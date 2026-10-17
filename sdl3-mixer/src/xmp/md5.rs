// Rust translation of src/md5.c and src/md5.h from libxmp 4.7.3 (as vendored
// by SDL_mixer).
// This code implements the MD5 message-digest algorithm. The algorithm is due
// to Ron Rivest. This code was written by Colin Plumb in 1993, no copyright
// is claimed. This code is in the public domain; do with it what you wish.
// This is an altered (translated) version of the original software.

/*
 * This code implements the MD5 message-digest algorithm.
 * The algorithm is due to Ron Rivest.	This code was
 * written by Colin Plumb in 1993, no copyright is claimed.
 * This code is in the public domain; do with it what you wish.
 *
 * Equivalent code is available from RSA Data Security, Inc.
 * This code has been tested against that, and is equivalent,
 * except that you don't need to include two pages of legalese
 * with every copy.
 *
 * To compute the message digest of a chunk of bytes, declare an
 * MD5Context structure, pass it to MD5Init, call MD5Update as
 * needed on buffers full of bytes, and then call MD5Final, which
 * will fill a supplied 16-byte array with the digest.
 */

pub(crate) const MD5_BLOCK_LENGTH: usize = 64;
pub(crate) const MD5_DIGEST_LENGTH: usize = 16;
pub(crate) const MD5_DIGEST_STRING_LENGTH: usize = MD5_DIGEST_LENGTH * 2 + 1;

/// Translation of `MD5_CTX`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Md5Ctx {
    state: [u32; 4],                /* state */
    count: u64,                     /* number of bits, mod 2^64 */
    buffer: [u8; MD5_BLOCK_LENGTH], /* input buffer */
}

static PADDING: [u8; MD5_BLOCK_LENGTH] = {
    let mut p = [0u8; MD5_BLOCK_LENGTH];
    p[0] = 0x80;
    p
};

/* The four core functions - F1 is optimized somewhat */

/* #define F1(x, y, z) (x & y | ~x & z) */
#[inline]
fn f1(x: u32, y: u32, z: u32) -> u32 {
    z ^ (x & (y ^ z))
}
#[inline]
fn f2(x: u32, y: u32, z: u32) -> u32 {
    f1(z, x, y)
}
#[inline]
fn f3(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}
#[inline]
fn f4(x: u32, y: u32, z: u32) -> u32 {
    y ^ (x | !z)
}

/* This is the central step in the MD5 algorithm. */
macro_rules! md5step {
    ($f:ident, $w:ident, $x:ident, $y:ident, $z:ident, $data:expr, $s:expr) => {
        $w = $w.wrapping_add($f($x, $y, $z)).wrapping_add($data);
        $w = $w.rotate_left($s);
        $w = $w.wrapping_add($x);
    };
}

/*
 * The core of the MD5 algorithm, this alters an existing MD5 hash to
 * reflect the addition of 16 longwords of new data.  MD5Update blocks
 * the data and converts bytes into longwords for this routine.
 */
fn md5_transform(state: &mut [u32; 4], block: &[u8]) {
    let mut inp = [0u32; MD5_BLOCK_LENGTH / 4];
    for (a, w) in inp.iter_mut().enumerate() {
        *w = u32::from_le_bytes([
            block[a * 4],
            block[a * 4 + 1],
            block[a * 4 + 2],
            block[a * 4 + 3],
        ]);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];

    md5step!(f1, a, b, c, d, inp[0].wrapping_add(0xd76aa478), 7);
    md5step!(f1, d, a, b, c, inp[1].wrapping_add(0xe8c7b756), 12);
    md5step!(f1, c, d, a, b, inp[2].wrapping_add(0x242070db), 17);
    md5step!(f1, b, c, d, a, inp[3].wrapping_add(0xc1bdceee), 22);
    md5step!(f1, a, b, c, d, inp[4].wrapping_add(0xf57c0faf), 7);
    md5step!(f1, d, a, b, c, inp[5].wrapping_add(0x4787c62a), 12);
    md5step!(f1, c, d, a, b, inp[6].wrapping_add(0xa8304613), 17);
    md5step!(f1, b, c, d, a, inp[7].wrapping_add(0xfd469501), 22);
    md5step!(f1, a, b, c, d, inp[8].wrapping_add(0x698098d8), 7);
    md5step!(f1, d, a, b, c, inp[9].wrapping_add(0x8b44f7af), 12);
    md5step!(f1, c, d, a, b, inp[10].wrapping_add(0xffff5bb1), 17);
    md5step!(f1, b, c, d, a, inp[11].wrapping_add(0x895cd7be), 22);
    md5step!(f1, a, b, c, d, inp[12].wrapping_add(0x6b901122), 7);
    md5step!(f1, d, a, b, c, inp[13].wrapping_add(0xfd987193), 12);
    md5step!(f1, c, d, a, b, inp[14].wrapping_add(0xa679438e), 17);
    md5step!(f1, b, c, d, a, inp[15].wrapping_add(0x49b40821), 22);

    md5step!(f2, a, b, c, d, inp[1].wrapping_add(0xf61e2562), 5);
    md5step!(f2, d, a, b, c, inp[6].wrapping_add(0xc040b340), 9);
    md5step!(f2, c, d, a, b, inp[11].wrapping_add(0x265e5a51), 14);
    md5step!(f2, b, c, d, a, inp[0].wrapping_add(0xe9b6c7aa), 20);
    md5step!(f2, a, b, c, d, inp[5].wrapping_add(0xd62f105d), 5);
    md5step!(f2, d, a, b, c, inp[10].wrapping_add(0x02441453), 9);
    md5step!(f2, c, d, a, b, inp[15].wrapping_add(0xd8a1e681), 14);
    md5step!(f2, b, c, d, a, inp[4].wrapping_add(0xe7d3fbc8), 20);
    md5step!(f2, a, b, c, d, inp[9].wrapping_add(0x21e1cde6), 5);
    md5step!(f2, d, a, b, c, inp[14].wrapping_add(0xc33707d6), 9);
    md5step!(f2, c, d, a, b, inp[3].wrapping_add(0xf4d50d87), 14);
    md5step!(f2, b, c, d, a, inp[8].wrapping_add(0x455a14ed), 20);
    md5step!(f2, a, b, c, d, inp[13].wrapping_add(0xa9e3e905), 5);
    md5step!(f2, d, a, b, c, inp[2].wrapping_add(0xfcefa3f8), 9);
    md5step!(f2, c, d, a, b, inp[7].wrapping_add(0x676f02d9), 14);
    md5step!(f2, b, c, d, a, inp[12].wrapping_add(0x8d2a4c8a), 20);

    md5step!(f3, a, b, c, d, inp[5].wrapping_add(0xfffa3942), 4);
    md5step!(f3, d, a, b, c, inp[8].wrapping_add(0x8771f681), 11);
    md5step!(f3, c, d, a, b, inp[11].wrapping_add(0x6d9d6122), 16);
    md5step!(f3, b, c, d, a, inp[14].wrapping_add(0xfde5380c), 23);
    md5step!(f3, a, b, c, d, inp[1].wrapping_add(0xa4beea44), 4);
    md5step!(f3, d, a, b, c, inp[4].wrapping_add(0x4bdecfa9), 11);
    md5step!(f3, c, d, a, b, inp[7].wrapping_add(0xf6bb4b60), 16);
    md5step!(f3, b, c, d, a, inp[10].wrapping_add(0xbebfbc70), 23);
    md5step!(f3, a, b, c, d, inp[13].wrapping_add(0x289b7ec6), 4);
    md5step!(f3, d, a, b, c, inp[0].wrapping_add(0xeaa127fa), 11);
    md5step!(f3, c, d, a, b, inp[3].wrapping_add(0xd4ef3085), 16);
    md5step!(f3, b, c, d, a, inp[6].wrapping_add(0x04881d05), 23);
    md5step!(f3, a, b, c, d, inp[9].wrapping_add(0xd9d4d039), 4);
    md5step!(f3, d, a, b, c, inp[12].wrapping_add(0xe6db99e5), 11);
    md5step!(f3, c, d, a, b, inp[15].wrapping_add(0x1fa27cf8), 16);
    md5step!(f3, b, c, d, a, inp[2].wrapping_add(0xc4ac5665), 23);

    md5step!(f4, a, b, c, d, inp[0].wrapping_add(0xf4292244), 6);
    md5step!(f4, d, a, b, c, inp[7].wrapping_add(0x432aff97), 10);
    md5step!(f4, c, d, a, b, inp[14].wrapping_add(0xab9423a7), 15);
    md5step!(f4, b, c, d, a, inp[5].wrapping_add(0xfc93a039), 21);
    md5step!(f4, a, b, c, d, inp[12].wrapping_add(0x655b59c3), 6);
    md5step!(f4, d, a, b, c, inp[3].wrapping_add(0x8f0ccc92), 10);
    md5step!(f4, c, d, a, b, inp[10].wrapping_add(0xffeff47d), 15);
    md5step!(f4, b, c, d, a, inp[1].wrapping_add(0x85845dd1), 21);
    md5step!(f4, a, b, c, d, inp[8].wrapping_add(0x6fa87e4f), 6);
    md5step!(f4, d, a, b, c, inp[15].wrapping_add(0xfe2ce6e0), 10);
    md5step!(f4, c, d, a, b, inp[6].wrapping_add(0xa3014314), 15);
    md5step!(f4, b, c, d, a, inp[13].wrapping_add(0x4e0811a1), 21);
    md5step!(f4, a, b, c, d, inp[4].wrapping_add(0xf7537e82), 6);
    md5step!(f4, d, a, b, c, inp[11].wrapping_add(0xbd3af235), 10);
    md5step!(f4, c, d, a, b, inp[2].wrapping_add(0x2ad7d2bb), 15);
    md5step!(f4, b, c, d, a, inp[9].wrapping_add(0xeb86d391), 21);

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

impl Md5Ctx {
    /*
     * Start MD5 accumulation.  Set bit count to 0 and buffer to mysterious
     * initialization constants.
     */
    /// Translation of `MD5Init()`.
    pub fn new() -> Md5Ctx {
        Md5Ctx {
            count: 0,
            state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476],
            buffer: [0; MD5_BLOCK_LENGTH],
        }
    }

    /*
     * Update context to reflect the concatenation of another buffer full
     * of bytes.
     */
    /// Translation of `MD5Update()`.
    pub fn update(&mut self, mut input: &[u8]) {
        let mut len = input.len();

        /* Check how many bytes we already have and how many more we need. */
        let mut have = ((self.count >> 3) & (MD5_BLOCK_LENGTH as u64 - 1)) as usize;
        let need = MD5_BLOCK_LENGTH - have;

        /* Update bitcount */
        self.count = self.count.wrapping_add((len as u64) << 3);

        if len >= need {
            if have != 0 {
                self.buffer[have..have + need].copy_from_slice(&input[..need]);
                let buffer = self.buffer;
                md5_transform(&mut self.state, &buffer);
                input = &input[need..];
                len -= need;
                have = 0;
            }

            /* Process data in MD5_BLOCK_LENGTH-byte chunks. */
            while len >= MD5_BLOCK_LENGTH {
                md5_transform(&mut self.state, input);
                input = &input[MD5_BLOCK_LENGTH..];
                len -= MD5_BLOCK_LENGTH;
            }
        }

        /* Handle any remaining bytes of data. */
        if len != 0 {
            self.buffer[have..have + len].copy_from_slice(&input[..len]);
        }
    }

    /*
     * Pad pad to 64-byte boundary with the bit pattern
     * 1 0* (64-bit count of bits processed, MSB-first)
     */
    /// Translation of `MD5Pad()`.
    fn pad(&mut self) {
        /* Convert count to 8 bytes in little endian order. */
        let count = self.count.to_le_bytes();

        /* Pad out to 56 mod 64. */
        let mut padlen =
            MD5_BLOCK_LENGTH - ((self.count >> 3) & (MD5_BLOCK_LENGTH as u64 - 1)) as usize;
        if padlen < 1 + 8 {
            padlen += MD5_BLOCK_LENGTH;
        }
        self.update(&PADDING[..padlen - 8]); /* padlen - 8 <= 64 */
        self.update(&count);
    }

    /*
     * Final wrapup--call MD5Pad, fill in digest and zero out ctx.
     */
    /// Translation of `MD5Final()`.
    pub fn finish(mut self) -> [u8; MD5_DIGEST_LENGTH] {
        let mut digest = [0u8; MD5_DIGEST_LENGTH];
        self.pad();
        for i in 0..4 {
            digest[i * 4..i * 4 + 4].copy_from_slice(&self.state[i].to_le_bytes());
        }
        digest
    }
}
