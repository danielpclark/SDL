// Rust translation of src/loaders/itsex.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Public domain IT sample decompressor by Olivier Lapicque
//
// Modified by Alice Rowan (2023-2024)- more or less complete
// rewrite of the input stream to add buffering.
//
// This translation is in the public domain too.

//! Impulse Tracker 2.14/2.15 compressed samples.

use super::super::hio::*;

/// `READ_BITS_MASK(n)`
#[inline]
fn read_bits_mask(n: i32) -> u32 {
    (1u32 << n as u32).wrapping_sub(1)
}

/// Translation of `struct it_stream` (`pos` indexes the block buffer).
#[derive(Default)]
struct ItStream {
    pos: usize,
    left: usize,
    bits: u32,
    num_bits: i32,
    err: i32,
}

/// Translation of `read_bits()`.
#[inline]
fn read_bits(in_: &mut ItStream, tmp: &[u8], mut n: i32) -> u32 {
    if n <= 0 || n >= 32 {
        /* Invalid shift value. */
        in_.err = -2;
        return 0;
    }

    let mut retval = in_.bits & read_bits_mask(n);

    if in_.num_bits < n {
        let offset = in_.num_bits;

        if in_.left == 0 {
            in_.err = EOF;
            return 0;
        }
        /* Buffer should be zero-padded to 4-byte alignment. */
        let p = in_.pos;
        in_.bits = tmp[p] as u32
            | ((tmp[p + 1] as u32) << 8)
            | ((tmp[p + 2] as u32) << 16)
            | ((tmp[p + 3] as u32) << 24);

        let used = in_.left.min(4);

        in_.num_bits = used as i32 * 8;
        in_.pos += 4;
        in_.left -= used;

        n -= offset;
        retval |= (in_.bits & read_bits_mask(n)) << offset as u32;
    }

    in_.bits >>= n as u32;
    in_.num_bits -= n;

    retval
}

/// Translation of `init_block()`.
#[inline]
fn init_block(in_: &mut ItStream, tmp: &mut [u8], tmplen: i32, src: &mut HioHandle<'_>) -> i32 {
    in_.pos = 0;
    in_.left = hio_read16l(src) as usize;
    in_.bits = 0;
    in_.num_bits = 0;
    in_.err = 0;

    /* tmp should be INT16_MAX rounded up to a multiple of 4 bytes long. */
    if (tmplen as i64) < ((in_.left + 4) & !3) as i64 {
        return -1;
    }
    if hio_read(tmp, 1, in_.left, src) < in_.left {
        return -1;
    }

    /* Zero pad to a multiple of 4 bytes for read_bits. */
    let mut i = in_.left;
    while i & 3 != 0 {
        tmp[i] = 0;
        i += 1;
    }

    0
}

/// Where the unpacking loops go after reading a value.
enum Step {
    UnpackByte,
    SkipByte,
    Next,
}

/// Translation of `itsex_decompress8()`.
pub(crate) fn itsex_decompress8(
    src: &mut HioHandle<'_>,
    dst: &mut [u8],
    mut len: i32,
    tmp: &mut [u8],
    tmplen: i32,
    it215: bool,
) -> i32 {
    let mut in_ = ItStream::default();
    let mut block_count: u32 = 0;
    let mut left: u8 = 0;
    let mut temp: u8 = 0;
    let mut temp2: u8 = 0;
    let mut dst_off = 0usize;

    while len != 0 {
        if block_count == 0 {
            block_count = 0x8000;
            left = 9;
            temp = 0;
            temp2 = 0;

            if init_block(&mut in_, tmp, tmplen, src) < 0 {
                return -1;
            }
        }

        let mut d = block_count;
        if d > len as u32 {
            d = len as u32;
        }

        /* Unpacking */
        let mut pos: u32 = 0;
        loop {
            let mut bits = read_bits(&mut in_, tmp, left as i32) as u16;
            if in_.err != 0 {
                return -1;
            }

            let step = 'b: {
                if left < 7 {
                    let i: u32 = 1 << (left - 1);
                    let j: u32 = (bits & 0xffff) as u32;
                    if i != j {
                        break 'b Step::UnpackByte;
                    }
                    bits = ((read_bits(&mut in_, tmp, 3) + 1) & 0xff) as u16;
                    if in_.err != 0 {
                        return -1;
                    }

                    left = if (bits as u8) < left {
                        bits as u8
                    } else {
                        ((bits + 1) & 0xff) as u8
                    };
                    break 'b Step::Next;
                }

                if left < 9 {
                    let i: u16 = (0xff >> (9 - left)) + 4;
                    let j: u16 = i.wrapping_sub(8);

                    if bits <= j || bits > i {
                        break 'b Step::UnpackByte;
                    }

                    bits -= j;
                    left = if ((bits & 0xff) as u8) < left {
                        (bits & 0xff) as u8
                    } else {
                        ((bits + 1) & 0xff) as u8
                    };
                    break 'b Step::Next;
                }

                if left >= 10 {
                    break 'b Step::SkipByte;
                }

                if bits >= 256 {
                    left = (bits.wrapping_add(1) as u8) & 0xff;
                    break 'b Step::Next;
                }

                Step::UnpackByte
            };

            match step {
                Step::UnpackByte => {
                    if left < 8 {
                        let shift = 8 - left;
                        let mut c = (bits << shift) as i8;
                        c >>= shift;
                        bits = c as u16;
                    }
                    bits = bits.wrapping_add(temp as u16);
                    temp = bits as u8;
                    temp2 = temp2.wrapping_add(temp);
                    dst[dst_off + pos as usize] = if it215 { temp2 } else { temp };
                    pos += 1;
                }
                Step::SkipByte => pos += 1,
                Step::Next => {
                    /* if (slen <= 0)
                    return -1 */
                }
            }

            if pos >= d {
                break;
            }
        }

        /* Move On */
        block_count -= d;
        len -= d as i32;
        dst_off += d as usize;
    }

    0
}

/// Translation of `itsex_decompress16()`: `dst` takes `len` native-endian
/// 16-bit samples.
pub(crate) fn itsex_decompress16(
    src: &mut HioHandle<'_>,
    dst: &mut [u8],
    mut len: i32,
    tmp: &mut [u8],
    tmplen: i32,
    it215: bool,
) -> i32 {
    let mut in_ = ItStream::default();
    let mut block_count: u32 = 0;
    let mut left: u8 = 0;
    let mut temp: i16 = 0;
    let mut temp2: i16 = 0;
    let mut dst_off = 0usize;

    while len != 0 {
        if block_count == 0 {
            block_count = 0x4000;
            left = 17;
            temp = 0;
            temp2 = 0;

            if init_block(&mut in_, tmp, tmplen, src) < 0 {
                return -1;
            }
        }

        let mut d = block_count;
        if d > len as u32 {
            d = len as u32;
        }

        /* Unpacking */
        let mut pos: u32 = 0;
        loop {
            let mut bits = read_bits(&mut in_, tmp, left as i32);
            if in_.err != 0 {
                return -1;
            }

            let step = 'b: {
                if left < 7 {
                    let i: u32 = 1 << (left - 1);
                    let j: u32 = bits;

                    if i != j {
                        break 'b Step::UnpackByte;
                    }

                    bits = read_bits(&mut in_, tmp, 4) + 1;
                    if in_.err != 0 {
                        return -1;
                    }

                    left = if ((bits & 0xff) as u8) < left {
                        (bits & 0xff) as u8
                    } else {
                        ((bits + 1) & 0xff) as u8
                    };
                    break 'b Step::Next;
                }

                if left < 17 {
                    let i: u32 = (0xffff >> (17 - left)) + 8;
                    let j: u32 = i.wrapping_sub(16) & 0xffff;

                    if bits <= j || bits > (i & 0xffff) {
                        break 'b Step::UnpackByte;
                    }

                    bits -= j;
                    left = if ((bits & 0xff) as u8) < left {
                        (bits & 0xff) as u8
                    } else {
                        ((bits + 1) & 0xff) as u8
                    };
                    break 'b Step::Next;
                }

                if left >= 18 {
                    break 'b Step::SkipByte;
                }

                if bits >= 0x10000 {
                    left = (bits.wrapping_add(1) as u8) & 0xff;
                    break 'b Step::Next;
                }

                Step::UnpackByte
            };

            match step {
                Step::UnpackByte => {
                    if left < 16 {
                        let shift = 16 - left;
                        let mut c = (bits << shift) as i16;
                        c >>= shift;
                        bits = c as i32 as u32;
                    }
                    bits = bits.wrapping_add(temp as i32 as u32);
                    temp = bits as i16;
                    temp2 = temp2.wrapping_add(temp);
                    let v = if it215 { temp2 } else { temp };
                    let o = (dst_off + pos as usize) * 2;
                    dst[o..o + 2].copy_from_slice(&v.to_ne_bytes());
                    pos += 1;
                }
                Step::SkipByte => pos += 1,
                Step::Next => {
                    /* if (slen <= 0)
                    return -1 */
                }
            }

            if pos >= d {
                break;
            }
        }

        /* Move On */
        block_count -= d;
        len -= d as i32;
        dst_off += d as usize;
        if len <= 0 {
            break;
        }
    }

    0
}
