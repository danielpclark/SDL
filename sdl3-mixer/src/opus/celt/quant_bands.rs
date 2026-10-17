// Rust translation of celt/quant_bands.h and celt/quant_bands.c from libopus
// 1.4 (https://opus-codec.org, at the revision SDL_mixer's external/opus
// pins).
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2009 Xiph.Org Foundation
// Written by Jean-Marc Valin
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Band energy decoding (the float build's tables; the encoder's
//! quantisation and `amp2Log2()` aren't translated).

use super::arch::max16;
use super::entcode::EcDec;
use super::laplace::ec_laplace_decode;
use super::modes::OpusCustomMode;
use super::rate::MAX_FINE_BITS;

/* Mean energy in each band quantized in Q4 and converted back to float */
pub(crate) static E_MEANS: [f32; 25] = [
    6.437500, 6.250000, 5.750000, 5.312500, 5.062500, 4.812500, 4.500000, 4.375000, 4.875000,
    4.687500, 4.562500, 4.437500, 4.875000, 4.625000, 4.312500, 4.500000, 4.375000, 4.625000,
    4.750000, 4.437500, 3.750000, 3.750000, 3.750000, 3.750000, 3.750000,
];

/* prediction coefficients: 0.9, 0.8, 0.65, 0.5 */
static PRED_COEF: [f32; 4] = [
    (29440.0 / 32768.0) as f32,
    (26112.0 / 32768.0) as f32,
    (21248.0 / 32768.0) as f32,
    (16384.0 / 32768.0) as f32,
];
static BETA_COEF: [f32; 4] = [
    (30147.0 / 32768.0) as f32,
    (22282.0 / 32768.0) as f32,
    (12124.0 / 32768.0) as f32,
    (6554.0 / 32768.0) as f32,
];
static BETA_INTRA: f32 = (4915.0 / 32768.0) as f32;

/*Parameters of the Laplace-like probability models used for the coarse energy.
There is one pair of parameters for each frame size, prediction type
(inter/intra), and band number.
The first number of each pair is the probability of 0, and the second is the
decay rate, both in Q8 precision.*/
#[rustfmt::skip]
static E_PROB_MODEL: [[[u8; 42]; 2]; 4] = [
   /*120 sample frames.*/
   [
      /*Inter*/
      [
          72, 127,  65, 129,  66, 128,  65, 128,  64, 128,  62, 128,  64, 128,
          64, 128,  92,  78,  92,  79,  92,  78,  90,  79, 116,  41, 115,  40,
         114,  40, 132,  26, 132,  26, 145,  17, 161,  12, 176,  10, 177,  11
      ],
      /*Intra*/
      [
          24, 179,  48, 138,  54, 135,  54, 132,  53, 134,  56, 133,  55, 132,
          55, 132,  61, 114,  70,  96,  74,  88,  75,  88,  87,  74,  89,  66,
          91,  67, 100,  59, 108,  50, 120,  40, 122,  37,  97,  43,  78,  50
      ]
   ],
   /*240 sample frames.*/
   [
      /*Inter*/
      [
          83,  78,  84,  81,  88,  75,  86,  74,  87,  71,  90,  73,  93,  74,
          93,  74, 109,  40, 114,  36, 117,  34, 117,  34, 143,  17, 145,  18,
         146,  19, 162,  12, 165,  10, 178,   7, 189,   6, 190,   8, 177,   9
      ],
      /*Intra*/
      [
          23, 178,  54, 115,  63, 102,  66,  98,  69,  99,  74,  89,  71,  91,
          73,  91,  78,  89,  86,  80,  92,  66,  93,  64, 102,  59, 103,  60,
         104,  60, 117,  52, 123,  44, 138,  35, 133,  31,  97,  38,  77,  45
      ]
   ],
   /*480 sample frames.*/
   [
      /*Inter*/
      [
          61,  90,  93,  60, 105,  42, 107,  41, 110,  45, 116,  38, 113,  38,
         112,  38, 124,  26, 132,  27, 136,  19, 140,  20, 155,  14, 159,  16,
         158,  18, 170,  13, 177,  10, 187,   8, 192,   6, 175,   9, 159,  10
      ],
      /*Intra*/
      [
          21, 178,  59, 110,  71,  86,  75,  85,  84,  83,  91,  66,  88,  73,
          87,  72,  92,  75,  98,  72, 105,  58, 107,  54, 115,  52, 114,  55,
         112,  56, 129,  51, 132,  40, 150,  33, 140,  29,  98,  35,  77,  42
      ]
   ],
   /*960 sample frames.*/
   [
      /*Inter*/
      [
          42, 121,  96,  66, 108,  43, 111,  40, 117,  44, 123,  32, 120,  36,
         119,  33, 127,  33, 134,  34, 139,  21, 147,  23, 152,  20, 158,  25,
         154,  26, 166,  21, 173,  16, 184,  13, 184,  10, 150,  13, 139,  15
      ],
      /*Intra*/
      [
          22, 178,  63, 114,  74,  82,  84,  83,  92,  82, 103,  62,  96,  72,
          96,  67, 101,  73, 107,  72, 113,  55, 118,  52, 125,  52, 118,  52,
         117,  55, 135,  49, 137,  39, 157,  32, 145,  29,  97,  33,  77,  40
      ]
   ]
];

static SMALL_ENERGY_ICDF: [u8; 3] = [2, 1, 0];

/// Translation of `unquant_coarse_energy()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn unquant_coarse_energy(
    m: &OpusCustomMode,
    start: i32,
    end: i32,
    old_ebands: &mut [f32],
    intra: i32,
    dec: &mut EcDec<'_>,
    c: i32,
    lm: i32,
) {
    let prob_model = &E_PROB_MODEL[lm as usize][intra as usize];
    let mut prev = [0.0f32; 2];
    let coef: f32;
    let beta: f32;

    if intra != 0 {
        coef = 0.0;
        beta = BETA_INTRA;
    } else {
        beta = BETA_COEF[lm as usize];
        coef = PRED_COEF[lm as usize];
    }

    let budget = (dec.storage * 8) as i32;

    /* Decode at a fixed coarse resolution */
    for i in start..end {
        for ci in 0..c as usize {
            let qi: i32;
            /* It would be better to express this invariant as a
            test on C at function entry, but that isn't enough
            to make the static analyzer happy. */
            let tell = dec.ec_tell();
            if budget - tell >= 15 {
                let pi = 2 * i.min(20) as usize;
                qi = ec_laplace_decode(
                    dec,
                    (prob_model[pi] as u32) << 7,
                    (prob_model[pi + 1] as i32) << 6,
                );
            } else if budget - tell >= 2 {
                let q = dec.ec_dec_icdf(&SMALL_ENERGY_ICDF, 2);
                qi = (q >> 1) ^ -(q & 1);
            } else if budget - tell >= 1 {
                qi = -dec.ec_dec_bit_logp(1);
            } else {
                qi = -1;
            }
            let q = qi as f32;

            let idx = i as usize + ci * m.nb_ebands as usize;
            old_ebands[idx] = max16(-9.0f32, old_ebands[idx]);
            let tmp = coef * old_ebands[idx] + prev[ci] + q;
            old_ebands[idx] = tmp;
            prev[ci] = prev[ci] + q - beta * q;
        }
    }
}

/// Translation of `unquant_fine_energy()`.
pub(crate) fn unquant_fine_energy(
    m: &OpusCustomMode,
    start: i32,
    end: i32,
    old_ebands: &mut [f32],
    fine_quant: &[i32],
    dec: &mut EcDec<'_>,
    c: i32,
) {
    /* Decode finer resolution */
    for i in start..end {
        let iu = i as usize;
        if fine_quant[iu] <= 0 {
            continue;
        }
        for ci in 0..c as usize {
            let q2 = dec.ec_dec_bits(fine_quant[iu] as u32) as i32;
            let offset =
                (q2 as f32 + 0.5f32) * (1i32 << (14 - fine_quant[iu])) as f32 * (1.0f32 / 16384.0)
                    - 0.5f32;
            old_ebands[iu + ci * m.nb_ebands as usize] += offset;
        }
    }
}

/// Translation of `unquant_energy_finalise()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn unquant_energy_finalise(
    m: &OpusCustomMode,
    start: i32,
    end: i32,
    old_ebands: &mut [f32],
    fine_quant: &[i32],
    fine_priority: &[i32],
    mut bits_left: i32,
    dec: &mut EcDec<'_>,
    c: i32,
) {
    /* Use up the remaining bits */
    for prio in 0..2 {
        let mut i = start;
        while i < end && bits_left >= c {
            let iu = i as usize;
            if fine_quant[iu] >= MAX_FINE_BITS || fine_priority[iu] != prio {
                i += 1;
                continue;
            }
            for ci in 0..c as usize {
                let q2 = dec.ec_dec_bits(1) as i32;
                let offset = (q2 as f32 - 0.5f32)
                    * (1i32 << (14 - fine_quant[iu] - 1)) as f32
                    * (1.0f32 / 16384.0);
                old_ebands[iu + ci * m.nb_ebands as usize] += offset;
                bits_left -= 1;
            }
            i += 1;
        }
    }
}
