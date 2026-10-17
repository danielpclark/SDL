// Rust translation of src/entropy_utils.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 1998 - 2013 Conifer Software.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The entropy decoder's metadata, its error limit, and the log2/exp2
//! conversions (not `log2buffer()`, `wp_log2s()` and `store_weight()`,
//! which only the encoder uses).

// This module contains the functions that process metadata blocks that are
// specific to the entropy decoder; these would be called any time a WavPack
// block was parsed. Additionally, it contains tables and functions that are
// common to both entropy coding and decoding. These are in a module separate
// from the actual entropy encoder (write_words.c) and decoder (read_words.c)
// so that if applications that just do a subset of the full WavPack reading
// and writing can link with a subset of the library.

use super::*;

///////////////////////////// local table storage ////////////////////////////

pub(crate) const BITSET: [u32; 32] = {
    let mut t = [0u32; 32];
    let mut i = 0;
    while i < 32 {
        t[i] = 1 << i;
        i += 1;
    }
    t
};

pub(crate) const BITMASK: [u32; 32] = {
    let mut t = [0u32; 32];
    let mut i = 0;
    while i < 31 {
        t[i] = (1 << i) - 1;
        i += 1;
    }
    t[31] = 0x7fffffff;
    t
};

pub(crate) const NBITS_TABLE: [u8; 256] = [
    0, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, // 0 - 15
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, // 16 - 31
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, // 32 - 47
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, // 48 - 63
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, // 64 - 79
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, // 80 - 95
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, // 96 - 111
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, // 112 - 127
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, // 128 - 143
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, // 144 - 159
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, // 160 - 175
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, // 176 - 191
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, // 192 - 207
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, // 208 - 223
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, // 224 - 239
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, // 240 - 255
];

static LOG2_TABLE: [u8; 256] = [
    0x00, 0x01, 0x03, 0x04, 0x06, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x10, 0x11, 0x12, 0x14, 0x15,
    0x16, 0x18, 0x19, 0x1a, 0x1c, 0x1d, 0x1e, 0x20, 0x21, 0x22, 0x24, 0x25, 0x26, 0x28, 0x29, 0x2a,
    0x2c, 0x2d, 0x2e, 0x2f, 0x31, 0x32, 0x33, 0x34, 0x36, 0x37, 0x38, 0x39, 0x3b, 0x3c, 0x3d, 0x3e,
    0x3f, 0x41, 0x42, 0x43, 0x44, 0x45, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4d, 0x4e, 0x4f, 0x50, 0x51,
    0x52, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63,
    0x64, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85,
    0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95,
    0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4,
    0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf, 0xb0, 0xb1, 0xb2, 0xb2,
    0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf, 0xc0, 0xc0,
    0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xcb, 0xcb, 0xcc, 0xcd, 0xce,
    0xcf, 0xd0, 0xd0, 0xd1, 0xd2, 0xd3, 0xd4, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd8, 0xd9, 0xda, 0xdb,
    0xdc, 0xdc, 0xdd, 0xde, 0xdf, 0xe0, 0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe4, 0xe5, 0xe6, 0xe7, 0xe7,
    0xe8, 0xe9, 0xea, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xee, 0xef, 0xf0, 0xf1, 0xf1, 0xf2, 0xf3, 0xf4,
    0xf4, 0xf5, 0xf6, 0xf7, 0xf7, 0xf8, 0xf9, 0xf9, 0xfa, 0xfb, 0xfc, 0xfc, 0xfd, 0xfe, 0xff, 0xff,
];

static EXP2_TABLE: [u8; 256] = [
    0x00, 0x01, 0x01, 0x02, 0x03, 0x03, 0x04, 0x05, 0x06, 0x06, 0x07, 0x08, 0x08, 0x09, 0x0a, 0x0b,
    0x0b, 0x0c, 0x0d, 0x0e, 0x0e, 0x0f, 0x10, 0x10, 0x11, 0x12, 0x13, 0x13, 0x14, 0x15, 0x16, 0x16,
    0x17, 0x18, 0x19, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1d, 0x1e, 0x1f, 0x20, 0x20, 0x21, 0x22, 0x23,
    0x24, 0x24, 0x25, 0x26, 0x27, 0x28, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2c, 0x2d, 0x2e, 0x2f, 0x30,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3a, 0x3b, 0x3c, 0x3d,
    0x3e, 0x3f, 0x40, 0x41, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x48, 0x49, 0x4a, 0x4b,
    0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a,
    0x5b, 0x5c, 0x5d, 0x5e, 0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79,
    0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x87, 0x88, 0x89, 0x8a,
    0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
    0x9c, 0x9d, 0x9f, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad,
    0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbc, 0xbd, 0xbe, 0xbf, 0xc0,
    0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc8, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf, 0xd0, 0xd2, 0xd3, 0xd4,
    0xd6, 0xd7, 0xd8, 0xd9, 0xdb, 0xdc, 0xdd, 0xde, 0xe0, 0xe1, 0xe2, 0xe4, 0xe5, 0xe6, 0xe8, 0xe9,
    0xea, 0xec, 0xed, 0xee, 0xf0, 0xf1, 0xf2, 0xf4, 0xf5, 0xf6, 0xf8, 0xf9, 0xfa, 0xfc, 0xfd, 0xff,
];

///////////////////////////// executable code ////////////////////////////////

/// `byteptr [i] + (byteptr [i + 1] << 8)`
#[inline]
fn le16(data: &[u8], i: usize) -> i32 {
    byte_at(data, i) as i32 + ((byte_at(data, i + 1) as i32) << 8)
}

// Read the median log2 values from the specified metadata structure, convert
// them back to 32-bit unsigned values and store them. If length is not
// exactly correct then we flag and return an error.

/// Translation of `read_entropy_vars()`.
pub(crate) fn read_entropy_vars(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    let byteptr = wpmd.bytes();

    if wpmd.byte_length
        != if (wps.wphdr.flags & MONO_DATA) != 0 {
            6
        } else {
            12
        }
    {
        return FALSE;
    }

    wps.w.c[0].median[0] = wp_exp2s(le16(byteptr, 0)) as u32;
    wps.w.c[0].median[1] = wp_exp2s(le16(byteptr, 2)) as u32;
    wps.w.c[0].median[2] = wp_exp2s(le16(byteptr, 4)) as u32;

    if (wps.wphdr.flags & MONO_DATA) == 0 {
        wps.w.c[1].median[0] = wp_exp2s(le16(byteptr, 6)) as u32;
        wps.w.c[1].median[1] = wp_exp2s(le16(byteptr, 8)) as u32;
        wps.w.c[1].median[2] = wp_exp2s(le16(byteptr, 10)) as u32;
    }

    TRUE
}

// Read the hybrid related values from the specified metadata structure, convert
// them back to their internal formats and store them. The extended profile
// stuff is not implemented yet, so return an error if we get more data than
// we know what to do with.

/// Translation of `read_hybrid_profile()`.
pub(crate) fn read_hybrid_profile(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    let data = wpmd.bytes();
    let mut byteptr = 0usize;
    let endptr = wpmd.byte_length as usize;
    let step = if (wps.wphdr.flags & MONO_DATA) != 0 {
        2
    } else {
        4
    };

    if (wps.wphdr.flags & HYBRID_BITRATE) != 0 {
        if byteptr + step > endptr {
            return FALSE;
        }

        wps.w.c[0].slow_level = wp_exp2s(le16(data, byteptr)) as u32;
        byteptr += 2;

        if (wps.wphdr.flags & MONO_DATA) == 0 {
            wps.w.c[1].slow_level = wp_exp2s(le16(data, byteptr)) as u32;
            byteptr += 2;
        }
    }

    if byteptr + step > endptr {
        return FALSE;
    }

    wps.w.bitrate_acc[0] = (le16(data, byteptr) as u32) << 16;
    byteptr += 2;

    if (wps.wphdr.flags & MONO_DATA) == 0 {
        wps.w.bitrate_acc[1] = (le16(data, byteptr) as u32) << 16;
        byteptr += 2;
    }

    if byteptr < endptr {
        if byteptr + step > endptr {
            return FALSE;
        }

        wps.w.bitrate_delta[0] = wp_exp2s(le16(data, byteptr) as i16 as i32) as u32;
        byteptr += 2;

        if (wps.wphdr.flags & MONO_DATA) == 0 {
            wps.w.bitrate_delta[1] = wp_exp2s(le16(data, byteptr) as i16 as i32) as u32;
            byteptr += 2;
        }

        if byteptr < endptr {
            return FALSE;
        }
    } else {
        wps.w.bitrate_delta[0] = 0;
        wps.w.bitrate_delta[1] = 0;
    }

    TRUE
}

// This function is called during both encoding and decoding of hybrid data to
// update the "error_limit" variable which determines the maximum sample error
// allowed in the main bitstream. In the HYBRID_BITRATE mode (which is the only
// currently implemented) this is calculated from the slow_level values and the
// bitrate accumulators. Note that the bitrate accumulators can be changing.

/// Translation of `update_error_limit()`.
pub(crate) fn update_error_limit(wps: &mut WavpackStream) {
    wps.w.bitrate_acc[0] = wps.w.bitrate_acc[0].wrapping_add(wps.w.bitrate_delta[0]);
    let mut bitrate_0 = (wps.w.bitrate_acc[0] >> 16) as i32;

    if (wps.wphdr.flags & MONO_DATA) != 0 {
        if (wps.wphdr.flags & HYBRID_BITRATE) != 0 {
            let slow_log_0 = (wps.w.c[0].slow_level.wrapping_add(SLO) >> SLS) as i32;

            if slow_log_0 - bitrate_0 > -0x100 {
                wps.w.c[0].error_limit = wp_exp2s(slow_log_0 - bitrate_0 + 0x100) as u32;
            } else {
                wps.w.c[0].error_limit = 0;
            }
        } else {
            wps.w.c[0].error_limit = wp_exp2s(bitrate_0) as u32;
        }
    } else {
        wps.w.bitrate_acc[1] = wps.w.bitrate_acc[1].wrapping_add(wps.w.bitrate_delta[1]);
        let mut bitrate_1 = (wps.w.bitrate_acc[1] >> 16) as i32;

        if (wps.wphdr.flags & HYBRID_BITRATE) != 0 {
            let slow_log_0 = (wps.w.c[0].slow_level.wrapping_add(SLO) >> SLS) as i32;
            let slow_log_1 = (wps.w.c[1].slow_level.wrapping_add(SLO) >> SLS) as i32;

            if (wps.wphdr.flags & HYBRID_BALANCE) != 0 {
                let balance = (slow_log_1 - slow_log_0 + bitrate_1 + 1) >> 1;

                if balance > bitrate_0 {
                    bitrate_1 = bitrate_0 * 2;
                    bitrate_0 = 0;
                } else if -balance > bitrate_0 {
                    bitrate_0 = bitrate_0 * 2;
                    bitrate_1 = 0;
                } else {
                    bitrate_1 = bitrate_0 + balance;
                    bitrate_0 = bitrate_0 - balance;
                }
            }

            if slow_log_0 - bitrate_0 > -0x100 {
                wps.w.c[0].error_limit = wp_exp2s(slow_log_0 - bitrate_0 + 0x100) as u32;
            } else {
                wps.w.c[0].error_limit = 0;
            }

            if slow_log_1 - bitrate_1 > -0x100 {
                wps.w.c[1].error_limit = wp_exp2s(slow_log_1 - bitrate_1 + 0x100) as u32;
            } else {
                wps.w.c[1].error_limit = 0;
            }
        } else {
            wps.w.c[0].error_limit = wp_exp2s(bitrate_0) as u32;
            wps.w.c[1].error_limit = wp_exp2s(bitrate_1) as u32;
        }
    }
}

// The concept of a base 2 logarithm is used in many parts of WavPack. It is
// a way of sufficiently accurately representing 32-bit signed and unsigned
// values storing only 16 bits (actually fewer). It is also used in the hybrid
// mode for quickly comparing the relative magnitude of large values (i.e.
// division) and providing smooth exponentials using only addition.

// These are not strict logarithms in that they become linear around zero and
// can therefore represent both zero and negative values. They have 8 bits
// of precision and in "roundtrip" conversions the total error never exceeds 1
// part in 225 except for the cases of +/-115 and +/-195 (which error by 1).

// This function returns the log2 for the specified 32-bit unsigned value.
// The maximum value allowed is about 0xff800000 and returns 8447.

/// Translation of `wp_log2()`.
pub(crate) fn wp_log2(mut avalue: u32) -> i32 {
    let dbits: i32;

    avalue = avalue.wrapping_add(avalue >> 9);
    if avalue < (1 << 8) {
        dbits = NBITS_TABLE[avalue as usize] as i32;
        (dbits << 8) + LOG2_TABLE[((avalue << (9 - dbits)) & 0xff) as usize] as i32
    } else {
        if avalue < (1 << 16) {
            dbits = NBITS_TABLE[(avalue >> 8) as usize] as i32 + 8;
        } else if avalue < (1 << 24) {
            dbits = NBITS_TABLE[(avalue >> 16) as usize] as i32 + 16;
        } else {
            dbits = NBITS_TABLE[(avalue >> 24) as usize] as i32 + 24;
        }

        (dbits << 8) + LOG2_TABLE[((avalue >> (dbits - 9)) & 0xff) as usize] as i32
    }
}

// This function returns the original integer represented by the supplied
// logarithm (at least within the provided accuracy). The log is signed,
// but since a full 32-bit value is returned this can be used for unsigned
// conversions as well (i.e. the input range is -8192 to +8447).

/// Translation of `wp_exp2s()`.
pub(crate) fn wp_exp2s(log: i32) -> i32 {
    if log < 0 {
        return !(wp_exp2s_positive(log.wrapping_neg()).wrapping_sub(1)) as i32;
    }

    wp_exp2s_positive(log) as i32
}

/// `wp_exp2s()` for a non-negative log (as `uint32_t`; the negative logs
/// that callers can give are in -0x10000..0, so the recursion upstream
/// makes is one level deep).
fn wp_exp2s_positive(mut log: i32) -> u32 {
    let value: u32 = EXP2_TABLE[(log & 0xff) as usize] as u32 | 0x100;

    log >>= 8;
    if log <= 9 {
        value.wrapping_shr((9 - log) as u32)
    } else {
        value << ((log - 9) & 0x1f)
    }
}

// These two functions convert internal weights (which are normally +/-1024)
// to and from an 8-bit signed character version for storage in metadata. The
// weights are clipped here in the case that they are outside that range.

/// Translation of `restore_weight()`.
pub(crate) fn restore_weight(weight: i8) -> i32 {
    let mut result = weight as i32 * 8;

    if result > 0 {
        result += (result + 64) >> 7;
    }

    result
}
