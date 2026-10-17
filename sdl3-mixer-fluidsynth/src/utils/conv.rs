/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: translated into Rust from fluid_conv.c and
// fluid_conv.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

use sdl3::stdlib::math::{log, pow};

use crate::gentables::{
    FLUID_CB2AMP_TAB, FLUID_CONCAVE_TAB, FLUID_CONVEX_TAB, FLUID_CT2HZ_TAB, FLUID_PAN_TAB,
};
use crate::utils::conv_tables::{FLUID_CB_AMP_SIZE, FLUID_VEL_CB_SIZE};
use crate::FluidReal;

/// `M_LN2`
pub(crate) const M_LN2: f64 = 0.69314718055994530941723212145818;
/// `FLUID_M_LN2`
pub(crate) const FLUID_M_LN2: FluidReal = M_LN2;

/*
 * Converts absolute cents to Hertz
 *
 * As per sfspec section 9.3:
 *
 * ABSOLUTE CENTS - An absolute logarithmic measure of frequency based on a
 * reference of MIDI key number scaled by 100.
 * A cent is 1/1200 of an octave [which is the twelve hundredth root of two],
 * and value 6900 is 440 Hz (A-440).
 *
 * Implemented below basically is the following:
 *   440 * 2^((cents-6900)/1200)
 * = 440 * 2^((int)((cents-6900)/1200)) * 2^(((int)cents-6900)%1200))
 * = 2^((int)((cents-6900)/1200)) * (440 * 2^(((int)cents-6900)%1200)))
 *                                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
 *                           This second factor is stored in the lookup table.
 *
 * The first factor can be implemented with a fast shift when the exponent
 * is always an int. This is the case when using 440/2^6 Hz rather than 440Hz
 * reference.
 */
/// Translation of `fluid_ct2hz_real()`.
pub(crate) fn fluid_ct2hz_real(cents: FluidReal) -> FluidReal {
    let mult: FluidReal;
    let mut fac: i32;
    let mut rem: i32;
    // (C's conversion of a double out of int's range is undefined; this
    // saturates.)
    let mut icents = cents as i32;

    // Offset the input argument by 300 cents, so that if cents==6900 +300 gives 7200 cents,
    // which is nicely divisible by 1200 and yields 2^6, which just perfectly matches the
    // 440/2^6 Hz reference value of our lookup table. Magic.
    icents = icents.wrapping_add(300);

    // don't use stdlib div() here, it turned out have poor performance
    fac = icents / 1200;

    // Calculating the modulo of negative numbers is implementation-defined behavior in C!
    // So make sure all these calculations are unsigned!
    if icents < 0 {
        rem = -((icents.wrapping_neg() as u32 % 1200) as i32);
    } else {
        rem = (icents as u32 % 1200) as i32;
    }

    // Handle negative remainder values
    if rem < 0 {
        // Bump rem back into positive range so that it fits our lookup table indexing below
        rem += 1200;
        // Since we've bumped the reminder up by a whole, we need to decrement the factor
        fac -= 1;
    }

    // Compute the first factor (mult) using powers of two
    if fac >= 0 {
        // Think of "mult" as the factor that we multiply (440/2^6)Hz with,
        // or in other words mult is the "first factor" of the above
        // functions comment.
        //
        // Assuming sizeof(uint)==4 this will give us a maximum range of
        // 32 * 1200cents - 300cents == 38100 cents == 29,527,900,160 Hz
        // which is much more than ever needed. For bigger values, just
        // safely wrap around (the & is just a replacement for the quick
        // modulo operation % 32).
        mult = (1u32 << (fac as u32 & 31)) as FluidReal;

        // don't use ldexp() either (poor performance)
        mult * FLUID_CT2HZ_TAB[rem as usize]
    } else {
        // Same mult calculation as for positive case, but here we need to take the inverse of it.
        // We could do:
        // mult = 1.0 / fast_shift
        // return mult * lookuptable
        // instead, already multiply in the lookup table here, do the division and save the multiplication
        mult = FLUID_CT2HZ_TAB[rem as usize]
            / (1u32 << ((fac.wrapping_neg()) as u32 & 31)) as FluidReal;
        mult
    }
}

/// Translation of `fluid_ct2hz()`.
pub(crate) fn fluid_ct2hz(mut cents: FluidReal) -> FluidReal {
    /* Filter fc limit: SF2.01 page 48 # 8 */
    if cents >= 13500.0 {
        cents = 13500.0; /* 20 kHz */
    } else if cents < 1500.0 {
        cents = 1500.0; /* 20 Hz */
    }

    fluid_ct2hz_real(cents)
}

/// Translation of `fluid_cb2amp()`.
///
/// in: a value between 0 and 1440, 0 is no attenuation
/// out: a value between 1 and 0
pub(crate) fn fluid_cb2amp(cb: FluidReal) -> FluidReal {
    /*
     * cb: an attenuation in 'centibels' (1/10 dB)
     * SF2.01 page 49 # 48 limits it to 144 dB.
     * 96 dB is reasonable for 16 bit systems, 144 would make sense for 24 bit.
     */

    /* minimum attenuation: 0 dB */
    if cb < 0.0 {
        /* Issue #1374: it seems that by using modLfoToVolEnv, the attenuation can become negative and
         * therefore the signal needs to be amplified.
         * In such a rare case, calculate the attenuation on the fly.
         *
         * This behavior is backed by the spec saying:
         * modLfoToVolume: "A positive number indicates a positive LFO excursion increases volume;
         * a negative number indicates a positive excursion decreases volume.
         * [...] For example, a value of 100 indicates that the volume will first rise ten dB, then fall ten dB."
         *
         * And in order to rise, a negative attenuation must be permitted.
         */
        return pow(10.0, cb / -200.0);
    }

    if cb >= FLUID_CB_AMP_SIZE as FluidReal {
        return 0.0;
    }

    // (a NaN gets here; C's (int) of it is undefined, this is index 0)
    FLUID_CB2AMP_TAB[cb as i32 as usize]
}

/// Translation of `fluid_tc2sec_delay()`.
pub(crate) fn fluid_tc2sec_delay(mut tc: FluidReal) -> FluidReal {
    /* SF2.01 section 8.1.2 items 21, 23, 25, 33
     * SF2.01 section 8.1.3 items 21, 23, 25, 33
     *
     * The most negative number indicates a delay of 0. Range is limited
     * from -12000 to 5000 */
    if tc <= -32768.0 {
        return 0.0;
    };

    if tc < -12000.0 {
        tc = -12000.0;
    }

    if tc > 5000.0 {
        tc = 5000.0;
    }

    fluid_tc2sec(tc)
}

/// Translation of `fluid_tc2sec_attack()`.
pub(crate) fn fluid_tc2sec_attack(mut tc: FluidReal) -> FluidReal {
    /* SF2.01 section 8.1.2 items 26, 34
     * SF2.01 section 8.1.3 items 26, 34
     * The most negative number indicates a delay of 0
     * Range is limited from -12000 to 8000 */
    if tc <= -32768.0 {
        return 0.0;
    };

    if tc < -12000.0 {
        tc = -12000.0;
    };

    if tc > 8000.0 {
        tc = 8000.0;
    };

    fluid_tc2sec(tc)
}

/// Translation of `fluid_tc2sec()`.
pub(crate) fn fluid_tc2sec(tc: FluidReal) -> FluidReal {
    /* No range checking here! */
    pow(2.0, tc / 1200.0)
}

/// Translation of `fluid_sec2tc()`: seconds to timecents.
pub(crate) fn fluid_sec2tc(sec: FluidReal) -> FluidReal {
    let mut res: FluidReal;
    if sec <= 0.0 {
        // would require a complex solution of fluid_tc2sec(), but this is real-only
        return -32768.0;
    }

    res = (1200.0 / M_LN2) * log(sec);
    if res < -32768.0 {
        res = -32768.0;
    }
    res
}

/// Translation of `fluid_tc2sec_release()`.
pub(crate) fn fluid_tc2sec_release(mut tc: FluidReal) -> FluidReal {
    /* SF2.01 section 8.1.2 items 30, 38
     * SF2.01 section 8.1.3 items 30, 38
     * No 'most negative number' rule here!
     * Range is limited from -12000 to 8000 */
    if tc <= -32768.0 {
        return 0.0;
    };

    if tc < -12000.0 {
        tc = -12000.0;
    };

    if tc > 8000.0 {
        tc = 8000.0;
    };

    fluid_tc2sec(tc)
}

/// Translation of `fluid_hz2ct()`: the inverse operation, converting from
/// Hertz to cents.
pub(crate) fn fluid_hz2ct(f: FluidReal) -> FluidReal {
    6900.0 + (1200.0 / FLUID_M_LN2) * log(f / 440.0)
}

/// Translation of `fluid_act2hz()`: convert from absolute cents to Hertz.
#[allow(dead_code)]
pub(crate) fn fluid_act2hz(c: f64) -> f64 {
    // do not use FLUID_POW, otherwise the unit tests will fail when compiled in single precision
    8.1757989156437073336828122976032719176391831357 * pow(2.0, c / 1200.0)
}

/// Translation of `fluid_pan()`.
pub(crate) fn fluid_pan(mut c: FluidReal, left: bool) -> FluidReal {
    if left {
        c = -c;
    }

    if c <= -500.0 {
        0.0
    } else if c >= 500.0 {
        1.0
    } else {
        // (a NaN gets here; C's (int) of it is undefined, this is 0)
        FLUID_PAN_TAB[(c as i32 + 500) as usize]
    }
}

/// Translation of `fluid_balance()`.
///
/// Return the amount of attenuation based on the balance for the specified
/// channel. If balance is negative (turned toward left channel, only the right
/// channel is attenuated. If balance is positive, only the left channel is
/// attenuated.
///
/// @params balance left/right balance, range [-960;960] in absolute centibels
/// @return amount of attenuation [0.0;1.0]
pub(crate) fn fluid_balance(mut balance: FluidReal, left: bool) -> FluidReal {
    /* This is the most common case */
    if balance == 0.0 {
        return 1.0;
    }

    if (left && balance < 0.0) || (!left && balance > 0.0) {
        return 1.0;
    }

    if balance < 0.0 {
        balance = -balance;
    }

    fluid_cb2amp(balance)
}

/// Translation of `fluid_concave()`.
pub(crate) fn fluid_concave(val: FluidReal) -> FluidReal {
    let ival = val as i32;
    if val < 0.0 {
        return 0.0;
    } else if ival >= FLUID_VEL_CB_SIZE as i32 - 1 {
        return FLUID_CONCAVE_TAB[FLUID_VEL_CB_SIZE - 1];
    }

    let ival = ival.max(0) as usize; // (a NaN: C's (int) is undefined)
    FLUID_CONCAVE_TAB[ival]
        + (FLUID_CONCAVE_TAB[ival + 1] - FLUID_CONCAVE_TAB[ival]) * (val - ival as FluidReal)
}

/// Translation of `fluid_convex()`.
pub(crate) fn fluid_convex(val: FluidReal) -> FluidReal {
    let ival = val as i32;
    if val < 0.0 {
        return 0.0;
    } else if ival >= FLUID_VEL_CB_SIZE as i32 - 1 {
        return FLUID_CONVEX_TAB[FLUID_VEL_CB_SIZE - 1];
    }

    let ival = ival.max(0) as usize; // (a NaN: C's (int) is undefined)
                                     // interpolation between convex steps: fixes bad sounds with modenv and filter cutoff
    FLUID_CONVEX_TAB[ival]
        + (FLUID_CONVEX_TAB[ival + 1] - FLUID_CONVEX_TAB[ival]) * (val - ival as FluidReal)
}
