// Rust translation of src/stdlib/SDL_murmur3.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// Public domain murmur3 32-bit hash algorithm
//
// Adapted from: https://en.wikipedia.org/wiki/MurmurHash

#[inline]
fn murmur_32_scramble(mut k: u32) -> u32 {
    k = k.wrapping_mul(0xcc9e2d51);
    k = k.rotate_left(15);
    k = k.wrapping_mul(0x1b873593);
    k
}

/// Calculate a 32-bit MurmurHash3 value for a block of data.
///
/// A seed may be specified, which changes the final results consistently, but
/// this does not work like `srand()`; the same `seed` must be used each time.
///
/// Translation of `SDL_murmur3_32()`.
pub fn murmur3_32(data: &[u8], seed: u32) -> u32 {
    let len = data.len();
    let mut hash = seed;

    // Read in groups of 4.
    let mut chunks = data.chunks_exact(4);
    for chunk in &mut chunks {
        // Equivalent to SDL_Swap32LE() on a memcpy'd Uint32.
        let k = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        hash ^= murmur_32_scramble(k);
        hash = hash.rotate_left(13);
        hash = hash.wrapping_mul(5).wrapping_add(0xe6546b64);
    }

    // Read the rest.
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let mut k: u32 = 0;
        for &b in rest.iter().rev() {
            k <<= 8;
            k |= b as u32;
        }
        // A swap is *not* necessary here because the preceding loop already
        // places the low bytes in the low places according to whatever endianness
        // we use. Swaps only apply when the memory is copied in a chunk.
        hash ^= murmur_32_scramble(k);
    }

    // Finalize.
    hash ^= len as u32;
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85ebca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2ae35);
    hash ^= hash >> 16;
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_vectors() {
        // Well-known MurmurHash3_x86_32 test vectors.
        assert_eq!(murmur3_32(b"", 0), 0);
        assert_eq!(murmur3_32(b"", 1), 0x514E28B7);
        assert_eq!(murmur3_32(b"", 0xffffffff), 0x81F16F39);
        assert_eq!(murmur3_32(&[0xFF, 0xFF, 0xFF, 0xFF], 0), 0x76293B50);
        assert_eq!(murmur3_32(&[0x21, 0x43, 0x65, 0x87], 0), 0xF55B516B);
        assert_eq!(murmur3_32(&[0x21, 0x43, 0x65], 0), 0x7E4A8634);
        assert_eq!(murmur3_32(&[0x21, 0x43], 0), 0xA0F7B07A);
        assert_eq!(murmur3_32(&[0x21], 0), 0x72661CF4);
        assert_eq!(murmur3_32(b"Hello, world!", 0x9747b28c), 0x24884CBA);
        assert_eq!(
            murmur3_32(b"The quick brown fox jumps over the lazy dog", 0x9747b28c),
            0x2FA826CD
        );
    }
}
