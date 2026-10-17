/*
    INT123_dct64.c: DCT64, the plain C version

    copyright ?-2006 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp
*/
// Modified 2026-10-17: translated into Rust from dct64.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

/*
 * Discrete Cosine Tansform (DCT) for subband synthesis
 *
 * -funroll-loops (for gcc) will remove the loops for better performance
 * using loops in the source-code enhances readabillity
 *
 *
 * TODO: write an optimized version for the down-sampling modes
 *       (in these modes the bands 16-31 (2:1) or 8-31 (4:1) are zero
 */

use crate::costabs::{COS16, COS32, COS4, COS64, COS8};

/// Translation of `INT123_dct64()`: `out0` and `out1` are written at
/// strides of `0x10`.
pub(crate) fn dct64(out0: &mut [f32], out1: &mut [f32], samples: &[f32]) {
    let mut bufs = [0.0f32; 64];

    {
        // (b1, b2 and bs index samples or bufs, as upstream's pointers do;
        // `*--b2` is `{ b2 -= 1; x[b2] }`)
        let mut bs = 0usize;
        let mut b1 = 0usize;
        let mut b2 = 32usize;
        let mut costab = 16usize; /* INT123_pnts[0]+16 */
        let cos = &COS64;

        for _i in 0..16 {
            b2 -= 1;
            bufs[bs] = samples[b1] + samples[b2];
            bs += 1;
            b1 += 1;
        }
        for _i in 0..16 {
            b2 -= 1;
            costab -= 1;
            bufs[bs] = (samples[b2] - samples[b1]) * cos[costab];
            bs += 1;
            b1 += 1;
        }

        // (from here on, b1 and b2 index bufs)
        let tmp = bufs;
        b1 = 0;
        let cos = &COS32;
        costab = 8; /* INT123_pnts[1]+8 */
        b2 = 16;

        {
            for _i in 0..8 {
                b2 -= 1;
                bufs[bs] = tmp[b1] + tmp[b2];
                bs += 1;
                b1 += 1;
            }
            for _i in 0..8 {
                b2 -= 1;
                costab -= 1;
                bufs[bs] = (tmp[b2] - tmp[b1]) * cos[costab];
                bs += 1;
                b1 += 1;
            }
            b2 += 32;
            costab += 8;
            for _i in 0..8 {
                b2 -= 1;
                bufs[bs] = tmp[b1] + tmp[b2];
                bs += 1;
                b1 += 1;
            }
            for _i in 0..8 {
                b2 -= 1;
                costab -= 1;
                bufs[bs] = (tmp[b1] - tmp[b2]) * cos[costab];
                bs += 1;
                b1 += 1;
            }
            b2 += 32;
            let _ = b2;
        }

        // (bs writes the lower half of bufs while b1 reads the upper half)
        bs = 0;
        let cos = &COS16;
        b2 = b1 + 8;

        for _j in 0..2 {
            for _i in 0..4 {
                b2 -= 1;
                bufs[bs] = bufs[b1] + bufs[b2];
                bs += 1;
                b1 += 1;
            }
            for i in (0..4).rev() {
                b2 -= 1;
                bufs[bs] = (bufs[b2] - bufs[b1]) * cos[i];
                bs += 1;
                b1 += 1;
            }
            b2 += 16;
            for _i in 0..4 {
                b2 -= 1;
                bufs[bs] = bufs[b1] + bufs[b2];
                bs += 1;
                b1 += 1;
            }
            for i in (0..4).rev() {
                b2 -= 1;
                bufs[bs] = (bufs[b1] - bufs[b2]) * cos[i];
                bs += 1;
                b1 += 1;
            }
            b2 += 16;
        }

        b1 = 0;
        let cos = &COS8;
        b2 = b1 + 4;

        for _j in 0..4 {
            b2 -= 1;
            bufs[bs] = bufs[b1] + bufs[b2];
            bs += 1;
            b1 += 1;
            b2 -= 1;
            bufs[bs] = bufs[b1] + bufs[b2];
            bs += 1;
            b1 += 1;
            b2 -= 1;
            bufs[bs] = (bufs[b2] - bufs[b1]) * cos[1];
            bs += 1;
            b1 += 1;
            b2 -= 1;
            bufs[bs] = (bufs[b2] - bufs[b1]) * cos[0];
            bs += 1;
            b1 += 1;
            b2 += 8;
            b2 -= 1;
            bufs[bs] = bufs[b1] + bufs[b2];
            bs += 1;
            b1 += 1;
            b2 -= 1;
            bufs[bs] = bufs[b1] + bufs[b2];
            bs += 1;
            b1 += 1;
            b2 -= 1;
            bufs[bs] = (bufs[b1] - bufs[b2]) * cos[1];
            bs += 1;
            b1 += 1;
            b2 -= 1;
            bufs[bs] = (bufs[b1] - bufs[b2]) * cos[0];
            bs += 1;
            b1 += 1;
            b2 += 8;
        }
        bs = 0;
        let costab = COS4[0];

        for _j in 0..8 {
            let mut v0 = bufs[b1];
            b1 += 1;
            let mut v1 = bufs[b1];
            b1 += 1;
            bufs[bs] = v0 + v1;
            bs += 1;
            bufs[bs] = (v0 - v1) * costab;
            bs += 1;
            v0 = bufs[b1];
            b1 += 1;
            v1 = bufs[b1];
            b1 += 1;
            bufs[bs] = v0 + v1;
            bs += 1;
            bufs[bs] = (v1 - v0) * costab;
            bs += 1;
        }
    }

    {
        let mut b1 = 0;
        for _i in 0..8 {
            bufs[b1 + 2] += bufs[b1 + 3];
            b1 += 4;
        }

        b1 = 0;
        for _i in 0..4 {
            bufs[b1 + 4] += bufs[b1 + 6];
            bufs[b1 + 6] += bufs[b1 + 5];
            bufs[b1 + 5] += bufs[b1 + 7];
            b1 += 8;
        }

        b1 = 0;
        for _i in 0..2 {
            bufs[b1 + 8] += bufs[b1 + 12];
            bufs[b1 + 12] += bufs[b1 + 10];
            bufs[b1 + 10] += bufs[b1 + 14];
            bufs[b1 + 14] += bufs[b1 + 9];
            bufs[b1 + 9] += bufs[b1 + 13];
            bufs[b1 + 13] += bufs[b1 + 11];
            bufs[b1 + 11] += bufs[b1 + 15];
            b1 += 16;
        }
    }

    out0[0x10 * 16] = bufs[0];
    out0[0x10 * 15] = bufs[16 + 0] + bufs[16 + 8];
    out0[0x10 * 14] = bufs[8];
    out0[0x10 * 13] = bufs[16 + 8] + bufs[16 + 4];
    out0[0x10 * 12] = bufs[4];
    out0[0x10 * 11] = bufs[16 + 4] + bufs[16 + 12];
    out0[0x10 * 10] = bufs[12];
    out0[0x10 * 9] = bufs[16 + 12] + bufs[16 + 2];
    out0[0x10 * 8] = bufs[2];
    out0[0x10 * 7] = bufs[16 + 2] + bufs[16 + 10];
    out0[0x10 * 6] = bufs[10];
    out0[0x10 * 5] = bufs[16 + 10] + bufs[16 + 6];
    out0[0x10 * 4] = bufs[6];
    out0[0x10 * 3] = bufs[16 + 6] + bufs[16 + 14];
    out0[0x10 * 2] = bufs[14];
    out0[0x10] = bufs[16 + 14] + bufs[16 + 1];
    out0[0] = bufs[1];

    out1[0] = bufs[1];
    out1[0x10] = bufs[16 + 1] + bufs[16 + 9];
    out1[0x10 * 2] = bufs[9];
    out1[0x10 * 3] = bufs[16 + 9] + bufs[16 + 5];
    out1[0x10 * 4] = bufs[5];
    out1[0x10 * 5] = bufs[16 + 5] + bufs[16 + 13];
    out1[0x10 * 6] = bufs[13];
    out1[0x10 * 7] = bufs[16 + 13] + bufs[16 + 3];
    out1[0x10 * 8] = bufs[3];
    out1[0x10 * 9] = bufs[16 + 3] + bufs[16 + 11];
    out1[0x10 * 10] = bufs[11];
    out1[0x10 * 11] = bufs[16 + 11] + bufs[16 + 7];
    out1[0x10 * 12] = bufs[7];
    out1[0x10 * 13] = bufs[16 + 7] + bufs[16 + 15];
    out1[0x10 * 14] = bufs[15];
    out1[0x10 * 15] = bufs[16 + 15];
}
