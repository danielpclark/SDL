// Rust translation of celt/rate.h and celt/rate.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2009 Xiph.Org Foundation
// Written by Jean-Marc Valin
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The bit allocation, as the decoder runs it (`encode` is 0: the
//! encoder's skip and stereo parameter decisions aren't translated; nor
//! is `compute_pulse_cache()`, which is only for `CUSTOM_MODES`).

use super::arch::{imax, imin};
use super::entcode::{celt_udiv, EcDec, BITRES};
use super::modes::OpusCustomMode;

#[allow(dead_code)]
pub(crate) const MAX_PSEUDO: i32 = 40;
pub(crate) const LOG_MAX_PSEUDO: i32 = 6;

#[allow(dead_code)]
pub(crate) const CELT_MAX_PULSES: i32 = 128;

pub(crate) const MAX_FINE_BITS: i32 = 8;

pub(crate) const FINE_OFFSET: i32 = 21;
pub(crate) const QTHETA_OFFSET: i32 = 4;
pub(crate) const QTHETA_OFFSET_TWOPHASE: i32 = 16;

/// Translation of `get_pulses()`.
#[inline]
pub(crate) fn get_pulses(i: i32) -> i32 {
    if i < 8 {
        i
    } else {
        (8 + (i & 7)) << ((i >> 3) - 1)
    }
}

/// `m->cache.bits + m->cache.index[LM*m->nbEBands+band]`.
#[inline]
pub(crate) fn cache_bits(m: &OpusCustomMode, lm1: i32, band: i32) -> &'static [u8] {
    let idx = m.cache.index[(lm1 * m.nb_ebands + band) as usize];
    // (the index is -1 only for the band sizes that are never split this
    // far, which aren't looked up.)
    &m.cache.bits[idx.max(0) as usize..]
}

/// Translation of `bits2pulses()`.
#[inline]
pub(crate) fn bits2pulses(m: &OpusCustomMode, band: i32, mut lm: i32, mut bits: i32) -> i32 {
    lm += 1;
    let cache = cache_bits(m, lm, band);

    let mut lo = 0i32;
    let mut hi = cache[0] as i32;
    bits -= 1;
    for _ in 0..LOG_MAX_PSEUDO {
        let mid = (lo + hi + 1) >> 1;
        /* OPT: Make sure this is implemented with a conditional move */
        if cache[mid as usize] as i32 >= bits {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    if bits
        - (if lo == 0 {
            -1
        } else {
            cache[lo as usize] as i32
        })
        <= cache[hi as usize] as i32 - bits
    {
        lo
    } else {
        hi
    }
}

/// Translation of `pulses2bits()`.
#[inline]
pub(crate) fn pulses2bits(m: &OpusCustomMode, band: i32, mut lm: i32, pulses: i32) -> i32 {
    lm += 1;
    let cache = cache_bits(m, lm, band);
    if pulses == 0 {
        0
    } else {
        cache[pulses as usize] as i32 + 1
    }
}

static LOG2_FRAC_TABLE: [u8; 24] = [
    0, 8, 13, 16, 19, 21, 23, 24, 26, 27, 28, 29, 30, 31, 32, 32, 33, 34, 34, 35, 36, 36, 37, 37,
];

const ALLOC_STEPS: i32 = 6;

#[allow(clippy::too_many_arguments)]
fn interp_bits2pulses(
    m: &OpusCustomMode,
    start: i32,
    end: i32,
    skip_start: i32,
    bits1: &[i32],
    bits2: &[i32],
    thresh: &[i32],
    cap: &[i32],
    mut total: i32,
    balance_out: &mut i32,
    skip_rsv: i32,
    intensity: &mut i32,
    mut intensity_rsv: i32,
    dual_stereo: &mut i32,
    mut dual_stereo_rsv: i32,
    bits: &mut [i32],
    ebits: &mut [i32],
    fine_priority: &mut [i32],
    c: i32,
    lm: i32,
    ec: &mut EcDec<'_>,
) -> i32 {
    let ebands = m.ebands;
    let eb = |j: i32| ebands[j as usize] as i32;
    let mut codedbands: i32;

    let alloc_floor = c << BITRES;
    let stereo = (c > 1) as i32;

    let log_m = lm << BITRES;
    let mut lo = 0;
    let mut hi = 1 << ALLOC_STEPS;
    for _ in 0..ALLOC_STEPS {
        let mid = (lo + hi) >> 1;
        let mut psum: i32 = 0;
        let mut done = false;
        let mut j = end;
        while j > start {
            j -= 1;
            let ju = j as usize;
            let tmp = bits1[ju] + (mid * bits2[ju] >> ALLOC_STEPS);
            if tmp >= thresh[ju] || done {
                done = true;
                /* Don't allocate more than we can actually use */
                psum += imin(tmp, cap[ju]);
            } else if tmp >= alloc_floor {
                psum += alloc_floor;
            }
        }
        if psum > total {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let mut psum: i32 = 0;
    /*printf ("interp bisection gave %d\n", lo);*/
    let mut done = false;
    let mut j = end;
    while j > start {
        j -= 1;
        let ju = j as usize;
        let mut tmp = bits1[ju] + (lo * bits2[ju] >> ALLOC_STEPS);
        if tmp < thresh[ju] && !done {
            if tmp >= alloc_floor {
                tmp = alloc_floor;
            } else {
                tmp = 0;
            }
        } else {
            done = true;
        }
        /* Don't allocate more than we can actually use */
        tmp = imin(tmp, cap[ju]);
        bits[ju] = tmp;
        psum += tmp;
    }

    /* Decide which bands to skip, working backwards from the end. */
    codedbands = end;
    loop {
        let j = codedbands - 1;
        let ju = j as usize;
        /* Never skip the first band, nor a band that has been boosted by
        dynalloc.
        In the first case, we'd be coding a bit to signal we're going to waste
        all the other bits.
        In the second case, we'd be coding a bit to redistribute all the bits
        we just signaled should be cocentrated in this band. */
        if j <= skip_start {
            /* Give the bit we reserved to end skipping back. */
            total += skip_rsv;
            break;
        }
        /*Figure out how many left-over bits we would be adding to this band.
        This can include bits we've stolen back from higher, skipped bands.*/
        // (C's int arithmetic, which wraps if left is negative.)
        let mut left = total.wrapping_sub(psum);
        let percoeff = celt_udiv(left as u32, (eb(codedbands) - eb(start)) as u32) as i32;
        left = left.wrapping_sub((eb(codedbands) - eb(start)).wrapping_mul(percoeff));
        let rem = imax(left.wrapping_sub(eb(j) - eb(start)), 0);
        let band_width = eb(codedbands) - eb(j);
        let mut band_bits = bits[ju]
            .wrapping_add(percoeff.wrapping_mul(band_width))
            .wrapping_add(rem);
        /*Only code a skip decision if we're above the threshold for this band.
        Otherwise it is force-skipped.
        This ensures that we have enough bits to code the skip flag.*/
        if band_bits >= imax(thresh[ju], alloc_floor + (1 << BITRES)) {
            if ec.ec_dec_bit_logp(1) != 0 {
                break;
            }
            /*We used a bit to skip this band.*/
            psum += 1 << BITRES;
            band_bits -= 1 << BITRES;
        }
        /*Reclaim the bits originally allocated to this band.*/
        psum -= bits[ju] + intensity_rsv;
        if intensity_rsv > 0 {
            intensity_rsv = LOG2_FRAC_TABLE[(j - start) as usize] as i32;
        }
        psum += intensity_rsv;
        if band_bits >= alloc_floor {
            /*If we have enough for a fine energy bit per channel, use it.*/
            psum += alloc_floor;
            bits[ju] = alloc_floor;
        } else {
            /*Otherwise this band gets nothing at all.*/
            bits[ju] = 0;
        }
        codedbands -= 1;
    }

    debug_assert!(codedbands > start);
    /* Code the intensity and dual stereo parameters. */
    if intensity_rsv > 0 {
        *intensity = start + ec.ec_dec_uint((codedbands + 1 - start) as u32) as i32;
    } else {
        *intensity = 0;
    }
    if *intensity <= start {
        total += dual_stereo_rsv;
        dual_stereo_rsv = 0;
    }
    if dual_stereo_rsv > 0 {
        *dual_stereo = ec.ec_dec_bit_logp(1);
    } else {
        *dual_stereo = 0;
    }

    /* Allocate the remaining bits */
    let mut left = total.wrapping_sub(psum);
    let percoeff = celt_udiv(left as u32, (eb(codedbands) - eb(start)) as u32) as i32;
    left = left.wrapping_sub((eb(codedbands) - eb(start)).wrapping_mul(percoeff));
    for j in start..codedbands {
        bits[j as usize] = bits[j as usize].wrapping_add(percoeff.wrapping_mul(eb(j + 1) - eb(j)));
    }
    for j in start..codedbands {
        let tmp = imin(left, eb(j + 1) - eb(j));
        bits[j as usize] = bits[j as usize].wrapping_add(tmp);
        left = left.wrapping_sub(tmp);
    }
    /*for (j=0;j<end;j++)printf("%d ", bits[j]);printf("\n");*/

    let mut balance: i32 = 0;
    let mut j = start;
    while j < codedbands {
        let ju = j as usize;
        let excess: i32;

        debug_assert!(bits[ju] >= 0);
        let n0 = eb(j + 1) - eb(j);
        let n = n0 << lm;
        let bit = bits[ju] + balance;

        let mut excess_v;
        if n > 1 {
            excess_v = imax(bit - cap[ju], 0);
            bits[ju] = bit - excess_v;

            /* Compensate for the extra DoF in stereo */
            let den = c * n
                + (if c == 2 && n > 2 && *dual_stereo == 0 && j < *intensity {
                    1
                } else {
                    0
                });

            let nclogn = den * (m.log_n[ju] as i32 + log_m);

            /* Offset for the number of fine bits by log2(N)/2 + FINE_OFFSET
            compared to their "fair share" of total/N */
            let mut offset = (nclogn >> 1) - den * FINE_OFFSET;

            /* N=2 is the only point that doesn't match the curve */
            if n == 2 {
                offset += den << BITRES >> 2;
            }

            /* Changing the offset for allocating the second and third
            fine energy bit */
            if bits[ju] + offset < den * 2 << BITRES {
                offset += nclogn >> 2;
            } else if bits[ju] + offset < den * 3 << BITRES {
                offset += nclogn >> 3;
            }

            /* Divide with rounding */
            ebits[ju] = imax(0, bits[ju] + offset + (den << (BITRES - 1)));
            ebits[ju] = (celt_udiv(ebits[ju] as u32, den as u32) >> BITRES) as i32;

            /* Make sure not to bust */
            if c * ebits[ju] > (bits[ju] >> BITRES) {
                ebits[ju] = bits[ju] >> stereo >> BITRES;
            }

            /* More than that is useless because that's about as far as PVQ can go */
            ebits[ju] = imin(ebits[ju], MAX_FINE_BITS);

            /* If we rounded down or capped this band, make it a candidate for the
            final fine energy pass */
            fine_priority[ju] = (ebits[ju] * (den << BITRES) >= bits[ju] + offset) as i32;

            /* Remove the allocated fine bits; the rest are assigned to PVQ */
            bits[ju] -= c * ebits[ju] << BITRES;
        } else {
            /* For N=1, all bits go to fine energy except for a single sign bit */
            excess_v = imax(0, bit - (c << BITRES));
            bits[ju] = bit - excess_v;
            ebits[ju] = 0;
            fine_priority[ju] = 1;
        }

        /* Fine energy can't take advantage of the re-balancing in
        quant_all_bands().
        Instead, do the re-balancing here.*/
        if excess_v > 0 {
            let extra_fine = imin(excess_v >> (stereo + BITRES), MAX_FINE_BITS - ebits[ju]);
            ebits[ju] += extra_fine;
            let extra_bits = extra_fine * c << BITRES;
            fine_priority[ju] = (extra_bits >= excess_v - balance) as i32;
            excess_v -= extra_bits;
        }
        excess = excess_v;
        balance = excess;

        debug_assert!(bits[ju] >= 0);
        debug_assert!(ebits[ju] >= 0);
        j += 1;
    }
    /* Save any remaining bits over the cap for the rebalancing in
    quant_all_bands(). */
    *balance_out = balance;

    /* The skipped bands use all their bits for fine energy. */
    while j < end {
        let ju = j as usize;
        ebits[ju] = bits[ju] >> stereo >> BITRES;
        debug_assert!(c * ebits[ju] << BITRES == bits[ju]);
        bits[ju] = 0;
        fine_priority[ju] = (ebits[ju] < 1) as i32;
        j += 1;
    }
    codedbands
}

/* Compute the pulse allocation, i.e. how many pulses will go in each
* band.
@param m mode
@param offsets Requested increase or decrease in the number of bits for
each band
@param total Number of bands
@param pulses Number of pulses per band (returned)
@return Total number of bits allocated
*/
/// Translation of `clt_compute_allocation()` (with `encode` 0).
#[allow(clippy::too_many_arguments)]
pub(crate) fn clt_compute_allocation(
    m: &OpusCustomMode,
    start: i32,
    end: i32,
    offsets: &[i32],
    cap: &[i32],
    alloc_trim: i32,
    intensity: &mut i32,
    dual_stereo: &mut i32,
    mut total: i32,
    balance: &mut i32,
    pulses: &mut [i32],
    ebits: &mut [i32],
    fine_priority: &mut [i32],
    c: i32,
    lm: i32,
    ec: &mut EcDec<'_>,
) -> i32 {
    let eb = |j: i32| m.ebands[j as usize] as i32;

    total = imax(total, 0);
    let len = m.nb_ebands;
    let mut skip_start = start;
    /* Reserve a bit to signal the end of manually skipped bands. */
    let skip_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
    total -= skip_rsv;
    /* Reserve bits for the intensity and dual stereo parameters. */
    let mut intensity_rsv = 0;
    let mut dual_stereo_rsv = 0;
    if c == 2 {
        intensity_rsv = LOG2_FRAC_TABLE[(end - start) as usize] as i32;
        if intensity_rsv > total {
            intensity_rsv = 0;
        } else {
            total -= intensity_rsv;
            dual_stereo_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
            total -= dual_stereo_rsv;
        }
    }
    let mut bits1 = [0i32; 21];
    let mut bits2 = [0i32; 21];
    let mut thresh = [0i32; 21];
    let mut trim_offset = [0i32; 21];

    for j in start..end {
        let ju = j as usize;
        /* Below this threshold, we're sure not to allocate any PVQ bits */
        thresh[ju] = imax(c << BITRES, (3 * (eb(j + 1) - eb(j)) << lm << BITRES) >> 4);
        /* Tilt of the allocation curve */
        trim_offset[ju] =
            c * (eb(j + 1) - eb(j)) * (alloc_trim - 5 - lm) * (end - j - 1) * (1 << (lm + BITRES))
                >> 6;
        /* Giving less resolution to single-coefficient bands because they get
        more benefit from having one coarse value per coefficient*/
        if (eb(j + 1) - eb(j)) << lm == 1 {
            trim_offset[ju] -= c << BITRES;
        }
    }
    let mut lo = 1;
    let mut hi = m.nb_alloc_vectors - 1;
    loop {
        let mut done = false;
        let mut psum = 0;
        let mid = (lo + hi) >> 1;
        let mut j = end;
        while j > start {
            j -= 1;
            let ju = j as usize;
            let n = eb(j + 1) - eb(j);
            let mut bitsj = (c * n * m.alloc_vectors[(mid * len + j) as usize] as i32) << lm >> 2;
            if bitsj > 0 {
                bitsj = imax(0, bitsj + trim_offset[ju]);
            }
            bitsj += offsets[ju];
            if bitsj >= thresh[ju] || done {
                done = true;
                /* Don't allocate more than we can actually use */
                psum += imin(bitsj, cap[ju]);
            } else if bitsj >= c << BITRES {
                psum += c << BITRES;
            }
        }
        if psum > total {
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
        /*printf ("lo = %d, hi = %d\n", lo, hi);*/
        if lo > hi {
            break;
        }
    }
    hi = lo;
    lo -= 1;
    /*printf ("interp between %d and %d\n", lo, hi);*/
    for j in start..end {
        let ju = j as usize;
        let n = eb(j + 1) - eb(j);
        let mut bits1j = (c * n * m.alloc_vectors[(lo * len + j) as usize] as i32) << lm >> 2;
        let mut bits2j = if hi >= m.nb_alloc_vectors {
            cap[ju]
        } else {
            (c * n * m.alloc_vectors[(hi * len + j) as usize] as i32) << lm >> 2
        };
        if bits1j > 0 {
            bits1j = imax(0, bits1j + trim_offset[ju]);
        }
        if bits2j > 0 {
            bits2j = imax(0, bits2j + trim_offset[ju]);
        }
        if lo > 0 {
            bits1j += offsets[ju];
        }
        bits2j += offsets[ju];
        if offsets[ju] > 0 {
            skip_start = j;
        }
        bits2j = imax(0, bits2j - bits1j);
        bits1[ju] = bits1j;
        bits2[ju] = bits2j;
    }
    interp_bits2pulses(
        m,
        start,
        end,
        skip_start,
        &bits1,
        &bits2,
        &thresh,
        cap,
        total,
        balance,
        skip_rsv,
        intensity,
        intensity_rsv,
        dual_stereo,
        dual_stereo_rsv,
        pulses,
        ebits,
        fine_priority,
        c,
        lm,
        ec,
    )
}
