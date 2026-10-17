// Rust translation of src/unpack_floats.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 1998 - 2013 Conifer Software.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Restoring 32-bit float data.

// This module deals with the restoration of floating-point data. Note that no
// floating point math is involved here...the values are only processed with
// the macros that directly access the mantissa, exponent, and sign fields.
// That's why we use the f32 type instead of the built-in float type.

use super::*;

/// Translation of `float_values()`.
pub(crate) fn float_values(wps: &mut WavpackStream, values: &mut [i32]) {
    let min_shifted_zeros = wps.float_min_shifted_zeros as i32;
    let max_shifted_ones = wps.float_max_shifted_ones as i32;
    let mut crc = wps.crc_x;

    if !bs_is_open(&wps.wvxbits) {
        float_values_nowvx(wps, values);
        return;
    }

    for value in values.iter_mut() {
        let mut shift_count = 0i32;
        let mut exp = wps.float_max_exp as i32;
        let mut outval: F32 = 0;
        let mut temp: u32;

        if *value == 0 {
            if (wps.float_flags & FLOAT_ZEROS_SENT) != 0 {
                if getbit(&mut wps.wvxbits) != 0 {
                    temp = getbits(23, &mut wps.wvxbits);
                    set_mantissa(&mut outval, temp as i32);

                    if exp >= 25 {
                        temp = getbits(8, &mut wps.wvxbits);
                        set_exponent(&mut outval, temp as i32);
                    }

                    set_sign(&mut outval, getbit(&mut wps.wvxbits) as i32);
                } else if (wps.float_flags & FLOAT_NEG_ZEROS) != 0 {
                    set_sign(&mut outval, getbit(&mut wps.wvxbits) as i32);
                }
            }
        } else {
            *value = ((*value as u32) << (wps.float_shift & 0x1f)) as i32;

            if *value < 0 {
                *value = value.wrapping_neg();
                set_sign(&mut outval, 1);
            }

            if *value == 0x1000000 {
                if getbit(&mut wps.wvxbits) != 0 {
                    temp = getbits(23, &mut wps.wvxbits);
                    set_mantissa(&mut outval, temp as i32);
                }

                set_exponent(&mut outval, 255);
            } else {
                if exp != 0 {
                    while (*value & 0x800000) == 0 && {
                        exp -= 1;
                        exp != 0
                    } {
                        shift_count += 1;
                        *value = ((*value as u32) << 1) as i32;
                    }
                }

                shift_count &= 0x1f;
                if shift_count != 0 {
                    if (wps.float_flags & FLOAT_SHIFT_ONES) != 0
                        || ((wps.float_flags & FLOAT_SHIFT_SAME) != 0
                            && getbit(&mut wps.wvxbits) != 0)
                    {
                        *value |= ((1u32 << shift_count) - 1) as i32;
                    } else if (wps.float_flags & FLOAT_SHIFT_SENT) != 0 {
                        let mask = ((1u32 << shift_count) - 1) as i32;
                        let mut num_zeros = 0;

                        if max_shifted_ones != 0 && shift_count > max_shifted_ones {
                            num_zeros = shift_count - max_shifted_ones;
                        }

                        if min_shifted_zeros > num_zeros {
                            num_zeros = if min_shifted_zeros > shift_count {
                                shift_count
                            } else {
                                min_shifted_zeros
                            };
                        }

                        shift_count -= num_zeros;
                        if shift_count > 0 {
                            temp = getbits(shift_count, &mut wps.wvxbits);
                            *value |= (temp.wrapping_shl(num_zeros as u32) as i32) & mask;
                        }
                    }
                }

                set_mantissa(&mut outval, *value);
                set_exponent(&mut outval, exp);
            }
        }

        crc = crc
            .wrapping_mul(27)
            .wrapping_add((get_mantissa(outval).wrapping_mul(9)) as u32)
            .wrapping_add((get_exponent(outval) * 3) as u32)
            .wrapping_add(get_sign(outval) as u32);
        *value = outval;
    }

    wps.crc_x = crc;
}

/// Translation of `float_values_nowvx()`.
fn float_values_nowvx(wps: &mut WavpackStream, values: &mut [i32]) {
    for value in values.iter_mut() {
        let mut shift_count = 0i32;
        let mut exp = wps.float_max_exp as i32;
        let mut outval: F32 = 0;

        if *value != 0 {
            *value = ((*value as u32) << (wps.float_shift & 0x1f)) as i32;

            if *value < 0 {
                *value = value.wrapping_neg();
                set_sign(&mut outval, 1);
            }

            if *value >= 0x1000000 {
                while (*value & 0xf000000) != 0 {
                    *value >>= 1;
                    exp += 1;
                }
            } else if exp != 0 {
                while (*value & 0x800000) == 0 && {
                    exp -= 1;
                    exp != 0
                } {
                    shift_count += 1;
                    *value = ((*value as u32) << 1) as i32;
                }

                shift_count &= 0x1f;
                if shift_count != 0 && (wps.float_flags & FLOAT_SHIFT_ONES) != 0 {
                    *value |= ((1u32 << shift_count) - 1) as i32;
                }
            }

            set_mantissa(&mut outval, *value);
            set_exponent(&mut outval, exp);
        }

        *value = outval;
    }
}
