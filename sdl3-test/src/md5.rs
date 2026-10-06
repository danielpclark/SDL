// Rust translation of src/test/SDL_test_md5.c and include/SDL3/SDL_test_md5.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! MD5 related functions of SDL test framework.
//!
//! ```text
//!  ***********************************************************************
//!  ** RSA Data Security, Inc. MD5 Message-Digest Algorithm              **
//!  ** Created: 2/17/90 RLR                                              **
//!  ** Revised: 1/91 SRD,AJ,BSK,JT Reference C ver., 7/10 constant corr. **
//!  ***********************************************************************
//!
//!  ***********************************************************************
//!  ** Copyright (C) 1990, RSA Data Security, Inc. All rights reserved.  **
//!  **                                                                   **
//!  ** License to copy and use this software is granted provided that    **
//!  ** it is identified as the "RSA Data Security, Inc. MD5 Message-     **
//!  ** Digest Algorithm" in all material mentioning or referencing this  **
//!  ** software or this function.                                        **
//!  **                                                                   **
//!  ** License is also granted to make and use derivative works          **
//!  ** provided that such works are identified as "derived from the RSA  **
//!  ** Data Security, Inc. MD5 Message-Digest Algorithm" in all          **
//!  ** material mentioning or referencing the derived work.              **
//!  **                                                                   **
//!  ** RSA Data Security, Inc. makes no representations concerning       **
//!  ** either the merchantability of this software or the suitability    **
//!  ** of this software for any particular purpose.  It is provided "as  **
//!  ** is" without express or implied warranty of any kind.              **
//!  **                                                                   **
//!  ** These notices must be retained in any copies of any part of this  **
//!  ** documentation and/or software.                                    **
//!  ***********************************************************************
//! ```
//!
//! ```text
//!  ***********************************************************************
//!  ** Header file for implementation of MD5                             **
//!  ** RSA Data Security, Inc. MD5 Message-Digest Algorithm              **
//!  ** Created: 2/17/90 RLR                                              **
//!  ** Revised: 12/27/90 SRD,AJ,BSK,JT Reference C version               **
//!  ** Revised (for MD5): RLR 4/27/91                                    **
//!  **   -- G modified to have y&~z instead of y&z                       **
//!  **   -- FF, GG, HH modified to add in last register done             **
//!  **   -- Access pattern: round 2 works mod 5, round 3 works mod 3     **
//!  **   -- distinct additive constant for each step                     **
//!  **   -- round 4 added, working mod 7                                 **
//!  ***********************************************************************
//!
//!  ***********************************************************************
//!  **  Message-digest routines:                                         **
//!  **  To form the message digest for a message M                       **
//!  **    (1) Initialize a context buffer mdContext using MD5Init        **
//!  **    (2) Call MD5Update on mdContext and M                          **
//!  **    (3) Call MD5Final on mdContext                                 **
//!  **  The message digest is now in mdContext->digest[0...15]           **
//!  ***********************************************************************
//! ```
//!
//! This module is derived from the RSA Data Security, Inc. MD5
//! Message-Digest Algorithm.

/* ------------ Definitions --------- */

/// A 32-bit type. Translation of `MD5UINT4`.
pub type Md5Uint4 = u32;

static MD5PADDING: [u8; 64] = {
    let mut padding = [0u8; 64];
    padding[0] = 0x80;
    padding
};

/* F, G, H and I are basic MD5 functions */
#[inline(always)]
fn f(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | ((!x) & z)
}
#[inline(always)]
fn g(x: u32, y: u32, z: u32) -> u32 {
    (x & z) | (y & (!z))
}
#[inline(always)]
fn h(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}
#[inline(always)]
fn i(x: u32, y: u32, z: u32) -> u32 {
    y ^ (x | (!z))
}

/* ROTATE_LEFT rotates x left n bits */
#[inline(always)]
fn rotate_left(x: u32, n: u32) -> u32 {
    x.rotate_left(n)
}

/* FF, GG, HH, and II transformations for rounds 1, 2, 3, and 4 */

/* Rotation is separate from addition to prevent recomputation */
macro_rules! step {
    ($fun:ident, $a:ident, $b:ident, $c:ident, $d:ident, $x:expr, $s:expr, $ac:expr) => {
        $a = $a
            .wrapping_add($fun($b, $c, $d))
            .wrapping_add($x)
            .wrapping_add($ac as Md5Uint4);
        $a = rotate_left($a, $s);
        $a = $a.wrapping_add($b);
    };
}

/// Data structure for MD5 (Message-Digest) computation. Translation of
/// `SDLTest_Md5Context`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Md5Context {
    /// number of _bits_ handled mod 2^64
    i: [Md5Uint4; 2],
    /// scratch buffer
    buf: [Md5Uint4; 4],
    /// input buffer
    in_: [u8; 64],
    /// actual digest after Md5Final call
    digest: [u8; 16],
}

impl Default for Md5Context {
    fn default() -> Self {
        Md5Context::new()
    }
}

impl Md5Context {
    /// The routine MD5Init initializes the message-digest context
    /// mdContext. All fields are set to zero. Make a new one before each new
    /// use. Translation of `SDLTest_Md5Init()`.
    pub fn new() -> Md5Context {
        Md5Context {
            i: [0, 0],

            /*
             * Load magic initialization constants.
             */
            buf: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476],
            in_: [0; 64],
            digest: [0; 16],
        }
    }

    /// The routine MD5Update updates the message-digest context to
    /// account for the presence of each of the characters inBuf[0..inLen-1]
    /// in the message whose digest is being computed. Translation of
    /// `SDLTest_Md5Update()`.
    pub fn update(&mut self, in_buf: &[u8]) {
        // (upstream's length is an unsigned int: longer data is taken in
        // pieces of that size)
        for piece in in_buf.chunks(u32::MAX as usize) {
            self.update_piece(piece);
        }
    }

    fn update_piece(&mut self, in_buf: &[u8]) {
        let in_len = in_buf.len() as u32;
        if in_len < 1 {
            return;
        }

        /*
         * compute number of bytes mod 64
         */
        let mut mdi = ((self.i[0] >> 3) & 0x3F) as usize;

        /*
         * update number of bits
         */
        if self.i[0].wrapping_add(in_len << 3) < self.i[0] {
            self.i[1] = self.i[1].wrapping_add(1);
        }
        self.i[0] = self.i[0].wrapping_add(in_len << 3);
        self.i[1] = self.i[1].wrapping_add(in_len >> 29);

        for &byte in in_buf {
            /*
             * add new character to buffer, increment mdi
             */
            self.in_[mdi] = byte;
            mdi += 1;

            /*
             * transform if necessary
             */
            if mdi == 0x40 {
                let input = self.input_words();
                transform(&mut self.buf, &input);
                mdi = 0;
            }
        }
    }

    /// The input buffer as little-endian words.
    fn input_words(&self) -> [Md5Uint4; 16] {
        let mut input = [0; 16];
        for (i, ii) in (0..16).zip((0..64).step_by(4)) {
            input[i] = ((self.in_[ii + 3] as Md5Uint4) << 24)
                | ((self.in_[ii + 2] as Md5Uint4) << 16)
                | ((self.in_[ii + 1] as Md5Uint4) << 8)
                | (self.in_[ii] as Md5Uint4);
        }
        input
    }

    /// The routine MD5Final terminates the message-digest computation and
    /// ends with the desired message digest, which it returns (and keeps,
    /// see [`digest`](Self::digest)). Translation of `SDLTest_Md5Final()`.
    pub fn finalize(&mut self) -> [u8; 16] {
        /*
         * save number of bits
         */
        let (bits_low, bits_high) = (self.i[0], self.i[1]);

        /*
         * compute number of bytes mod 64
         */
        let mdi = ((self.i[0] >> 3) & 0x3F) as usize;

        /*
         * pad out to 56 mod 64
         */
        let pad_len = if mdi < 56 { 56 - mdi } else { 120 - mdi };
        self.update(&MD5PADDING[..pad_len]);

        /*
         * append length in bits and transform
         */
        let mut input = self.input_words();
        input[14] = bits_low;
        input[15] = bits_high;
        transform(&mut self.buf, &input);

        /*
         * store buffer in digest
         */
        for (i, ii) in (0..4).zip((0..16).step_by(4)) {
            self.digest[ii] = (self.buf[i] & 0xFF) as u8;
            self.digest[ii + 1] = ((self.buf[i] >> 8) & 0xFF) as u8;
            self.digest[ii + 2] = ((self.buf[i] >> 16) & 0xFF) as u8;
            self.digest[ii + 3] = ((self.buf[i] >> 24) & 0xFF) as u8;
        }
        self.digest
    }

    /// The digest after [`finalize`](Self::finalize) (zeros before).
    /// Translation of reading `SDLTest_Md5Context.digest`.
    pub fn digest(&self) -> [u8; 16] {
        self.digest
    }
}

/// Basic MD5 step. Transforms buf based on in. Translation of
/// `SDLTest_Md5Transform()`.
fn transform(buf: &mut [Md5Uint4; 4], input: &[Md5Uint4; 16]) {
    let (mut a, mut b, mut c, mut d) = (buf[0], buf[1], buf[2], buf[3]);

    /*
     * Round 1
     */
    const S11: u32 = 7;
    const S12: u32 = 12;
    const S13: u32 = 17;
    const S14: u32 = 22;
    step!(f, a, b, c, d, input[0], S11, 3614090360u32); /* 1 */
    step!(f, d, a, b, c, input[1], S12, 3905402710u32); /* 2 */
    step!(f, c, d, a, b, input[2], S13, 606105819u32); /* 3 */
    step!(f, b, c, d, a, input[3], S14, 3250441966u32); /* 4 */
    step!(f, a, b, c, d, input[4], S11, 4118548399u32); /* 5 */
    step!(f, d, a, b, c, input[5], S12, 1200080426u32); /* 6 */
    step!(f, c, d, a, b, input[6], S13, 2821735955u32); /* 7 */
    step!(f, b, c, d, a, input[7], S14, 4249261313u32); /* 8 */
    step!(f, a, b, c, d, input[8], S11, 1770035416u32); /* 9 */
    step!(f, d, a, b, c, input[9], S12, 2336552879u32); /* 10 */
    step!(f, c, d, a, b, input[10], S13, 4294925233u32); /* 11 */
    step!(f, b, c, d, a, input[11], S14, 2304563134u32); /* 12 */
    step!(f, a, b, c, d, input[12], S11, 1804603682u32); /* 13 */
    step!(f, d, a, b, c, input[13], S12, 4254626195u32); /* 14 */
    step!(f, c, d, a, b, input[14], S13, 2792965006u32); /* 15 */
    step!(f, b, c, d, a, input[15], S14, 1236535329u32); /* 16 */

    /*
     * Round 2
     */
    const S21: u32 = 5;
    const S22: u32 = 9;
    const S23: u32 = 14;
    const S24: u32 = 20;
    step!(g, a, b, c, d, input[1], S21, 4129170786u32); /* 17 */
    step!(g, d, a, b, c, input[6], S22, 3225465664u32); /* 18 */
    step!(g, c, d, a, b, input[11], S23, 643717713u32); /* 19 */
    step!(g, b, c, d, a, input[0], S24, 3921069994u32); /* 20 */
    step!(g, a, b, c, d, input[5], S21, 3593408605u32); /* 21 */
    step!(g, d, a, b, c, input[10], S22, 38016083u32); /* 22 */
    step!(g, c, d, a, b, input[15], S23, 3634488961u32); /* 23 */
    step!(g, b, c, d, a, input[4], S24, 3889429448u32); /* 24 */
    step!(g, a, b, c, d, input[9], S21, 568446438u32); /* 25 */
    step!(g, d, a, b, c, input[14], S22, 3275163606u32); /* 26 */
    step!(g, c, d, a, b, input[3], S23, 4107603335u32); /* 27 */
    step!(g, b, c, d, a, input[8], S24, 1163531501u32); /* 28 */
    step!(g, a, b, c, d, input[13], S21, 2850285829u32); /* 29 */
    step!(g, d, a, b, c, input[2], S22, 4243563512u32); /* 30 */
    step!(g, c, d, a, b, input[7], S23, 1735328473u32); /* 31 */
    step!(g, b, c, d, a, input[12], S24, 2368359562u32); /* 32 */

    /*
     * Round 3
     */
    const S31: u32 = 4;
    const S32: u32 = 11;
    const S33: u32 = 16;
    const S34: u32 = 23;
    step!(h, a, b, c, d, input[5], S31, 4294588738u32); /* 33 */
    step!(h, d, a, b, c, input[8], S32, 2272392833u32); /* 34 */
    step!(h, c, d, a, b, input[11], S33, 1839030562u32); /* 35 */
    step!(h, b, c, d, a, input[14], S34, 4259657740u32); /* 36 */
    step!(h, a, b, c, d, input[1], S31, 2763975236u32); /* 37 */
    step!(h, d, a, b, c, input[4], S32, 1272893353u32); /* 38 */
    step!(h, c, d, a, b, input[7], S33, 4139469664u32); /* 39 */
    step!(h, b, c, d, a, input[10], S34, 3200236656u32); /* 40 */
    step!(h, a, b, c, d, input[13], S31, 681279174u32); /* 41 */
    step!(h, d, a, b, c, input[0], S32, 3936430074u32); /* 42 */
    step!(h, c, d, a, b, input[3], S33, 3572445317u32); /* 43 */
    step!(h, b, c, d, a, input[6], S34, 76029189u32); /* 44 */
    step!(h, a, b, c, d, input[9], S31, 3654602809u32); /* 45 */
    step!(h, d, a, b, c, input[12], S32, 3873151461u32); /* 46 */
    step!(h, c, d, a, b, input[15], S33, 530742520u32); /* 47 */
    step!(h, b, c, d, a, input[2], S34, 3299628645u32); /* 48 */

    /*
     * Round 4
     */
    const S41: u32 = 6;
    const S42: u32 = 10;
    const S43: u32 = 15;
    const S44: u32 = 21;
    step!(i, a, b, c, d, input[0], S41, 4096336452u32); /* 49 */
    step!(i, d, a, b, c, input[7], S42, 1126891415u32); /* 50 */
    step!(i, c, d, a, b, input[14], S43, 2878612391u32); /* 51 */
    step!(i, b, c, d, a, input[5], S44, 4237533241u32); /* 52 */
    step!(i, a, b, c, d, input[12], S41, 1700485571u32); /* 53 */
    step!(i, d, a, b, c, input[3], S42, 2399980690u32); /* 54 */
    step!(i, c, d, a, b, input[10], S43, 4293915773u32); /* 55 */
    step!(i, b, c, d, a, input[1], S44, 2240044497u32); /* 56 */
    step!(i, a, b, c, d, input[8], S41, 1873313359u32); /* 57 */
    step!(i, d, a, b, c, input[15], S42, 4264355552u32); /* 58 */
    step!(i, c, d, a, b, input[6], S43, 2734768916u32); /* 59 */
    step!(i, b, c, d, a, input[13], S44, 1309151649u32); /* 60 */
    step!(i, a, b, c, d, input[4], S41, 4149444226u32); /* 61 */
    step!(i, d, a, b, c, input[11], S42, 3174756917u32); /* 62 */
    step!(i, c, d, a, b, input[2], S43, 718787259u32); /* 63 */
    step!(i, b, c, d, a, input[9], S44, 3951481745u32); /* 64 */

    buf[0] = buf[0].wrapping_add(a);
    buf[1] = buf[1].wrapping_add(b);
    buf[2] = buf[2].wrapping_add(c);
    buf[3] = buf[3].wrapping_add(d);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md5(data: &[u8]) -> String {
        let mut context = Md5Context::new();
        context.update(data);
        let digest = context.finalize();
        assert_eq!(context.digest(), digest);
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn rfc1321_test_suite() {
        // The test suite of RFC 1321, appendix A.5.
        assert_eq!(md5(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5(b"a"), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(md5(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(md5(b"message digest"), "f96b697d7cb7938d525a2f31aaf161d0");
        assert_eq!(
            md5(b"abcdefghijklmnopqrstuvwxyz"),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
        assert_eq!(
            md5(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
        assert_eq!(
            md5(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            ),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    #[test]
    fn padding_boundaries_and_pieces() {
        // Lengths around the 56-byte padding boundary and the 64-byte block,
        // whole and fed in pieces.
        let data: Vec<u8> = (0..200u32).map(|i| (i * 7 + 3) as u8).collect();
        for len in [55, 56, 57, 63, 64, 65, 119, 120, 121, 128, 200] {
            let whole = md5(&data[..len]);
            let mut context = Md5Context::new();
            for piece in data[..len].chunks(13) {
                context.update(piece);
            }
            let digest = context.finalize();
            let pieces: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(whole, pieces, "length {len}");
        }
        // A million 'a's.
        assert_eq!(
            md5(&vec![b'a'; 1_000_000]),
            "7707d6ae4e027c70eea2a935c2296f21"
        );
    }
}
