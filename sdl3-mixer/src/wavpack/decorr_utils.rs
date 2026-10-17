// Rust translation of src/decorr_utils.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 1998 - 2013 Conifer Software.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The decorrelator's metadata.

// This module contains the functions that process metadata blocks that are
// specific to the decorrelator. These would be called any time a WavPack
// block was parsed. These are in a module separate from the actual unpack
// decorrelation code (unpack.c) so that if an application just wants to get
// information from WavPack files (rather than actually decoding audio) then
// less code needs to be linked.

use super::entropy_utils::{restore_weight, wp_exp2s};
use super::*;

///////////////////////////// executable code ////////////////////////////////

/// `wp_exp2s ((int16_t)(byteptr [i] + (byteptr [i + 1] << 8)))`
#[inline]
fn exp2s16(data: &[u8], i: usize) -> i32 {
    wp_exp2s((byte_at(data, i) as i32 + ((byte_at(data, i + 1) as i32) << 8)) as i16 as i32)
}

// Read decorrelation terms from specified metadata block into the
// decorr_passes array. The terms range from -3 to 8, plus 17 & 18;
// other values are reserved and generate errors for now. The delta
// ranges from 0 to 7 with all values valid. Note that the terms are
// stored in the opposite order in the decorr_passes array compared
// to packing.

/// Translation of `read_decorr_terms()`.
pub(crate) fn read_decorr_terms(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    let mut termcnt = wpmd.byte_length;
    let data = wpmd.bytes();
    let mut byteptr = 0usize;

    if termcnt > MAX_NTERMS as i32 {
        return FALSE;
    }

    wps.num_terms = termcnt;

    let mut dpp = termcnt - 1;
    while termcnt > 0 {
        termcnt -= 1;
        let b = byte_at(data, byteptr);
        byteptr += 1;
        let pass = &mut wps.decorr_passes[dpp as usize];
        pass.term = (b & 0x1f) as i32 - 5;
        pass.delta = ((b >> 5) & 0x7) as i32;

        if pass.term == 0
            || pass.term < -3
            || (pass.term > MAX_TERM && pass.term < 17)
            || pass.term > 18
            || ((wps.wphdr.flags & MONO_DATA) != 0 && pass.term < 0)
        {
            return FALSE;
        }
        dpp -= 1;
    }

    TRUE
}

// Read decorrelation weights from specified metadata block into the
// decorr_passes array. The weights range +/-1024, but are rounded and
// truncated to fit in signed chars for metadata storage. Weights are
// separate for the two channels and are specified from the "last" term
// (first during encode). Unspecified weights are set to zero.

/// Translation of `read_decorr_weights()`.
pub(crate) fn read_decorr_weights(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    let mut termcnt = wpmd.byte_length;
    let data = wpmd.bytes();
    let mut byteptr = 0usize;

    if (wps.wphdr.flags & MONO_DATA) == 0 {
        termcnt /= 2;
    }

    if termcnt > wps.num_terms {
        return FALSE;
    }

    for dpp in wps.decorr_passes[..wps.num_terms as usize].iter_mut() {
        dpp.weight_a = 0;
        dpp.weight_b = 0;
    }

    let mut dpp = wps.num_terms;
    loop {
        dpp -= 1;
        if dpp < 0 {
            break;
        }
        if termcnt == 0 {
            break;
        }
        termcnt -= 1;
        let pass = &mut wps.decorr_passes[dpp as usize];
        pass.weight_a = restore_weight(byte_at(data, byteptr) as i8);
        byteptr += 1;

        if (wps.wphdr.flags & MONO_DATA) == 0 {
            pass.weight_b = restore_weight(byte_at(data, byteptr) as i8);
            byteptr += 1;
        }
    }

    TRUE
}

// Read decorrelation samples from specified metadata block into the
// decorr_passes array. The samples are signed 32-bit values, but are
// converted to signed log2 values for storage in metadata. Values are
// stored for both channels and are specified from the "last" term
// (first during encode) with unspecified samples set to zero. The
// number of samples stored varies with the actual term value, so
// those must obviously come first in the metadata.

/// Translation of `read_decorr_samples()`.
pub(crate) fn read_decorr_samples(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    let data = wpmd.bytes();
    let mut byteptr = 0usize;
    let endptr = wpmd.byte_length as usize;
    let mono = (wps.wphdr.flags & MONO_DATA) != 0;

    for dpp in wps.decorr_passes[..wps.num_terms as usize].iter_mut() {
        dpp.samples_a = [0; MAX_TERM as usize];
        dpp.samples_b = [0; MAX_TERM as usize];
    }

    if wps.wphdr.version == 0x402 && (wps.wphdr.flags & HYBRID_FLAG) != 0 {
        if byteptr + if mono { 2 } else { 4 } > endptr {
            return FALSE;
        }

        wps.dc.error[0] = exp2s16(data, byteptr);
        byteptr += 2;

        if !mono {
            wps.dc.error[1] = exp2s16(data, byteptr);
            byteptr += 2;
        }
    }

    let mut dpp = wps.num_terms as usize;
    while dpp > 0 && byteptr < endptr {
        dpp -= 1;
        let pass = &mut wps.decorr_passes[dpp];
        if pass.term > MAX_TERM {
            if byteptr + if mono { 4 } else { 8 } > endptr {
                return FALSE;
            }

            pass.samples_a[0] = exp2s16(data, byteptr);
            pass.samples_a[1] = exp2s16(data, byteptr + 2);
            byteptr += 4;

            if !mono {
                pass.samples_b[0] = exp2s16(data, byteptr);
                pass.samples_b[1] = exp2s16(data, byteptr + 2);
                byteptr += 4;
            }
        } else if pass.term < 0 {
            if byteptr + 4 > endptr {
                return FALSE;
            }

            pass.samples_a[0] = exp2s16(data, byteptr);
            pass.samples_b[0] = exp2s16(data, byteptr + 2);
            byteptr += 4;
        } else {
            let mut m = 0usize;
            let mut cnt = pass.term;

            while cnt > 0 {
                cnt -= 1;
                if byteptr + if mono { 2 } else { 4 } > endptr {
                    return FALSE;
                }

                pass.samples_a[m] = exp2s16(data, byteptr);
                byteptr += 2;

                if !mono {
                    pass.samples_b[m] = exp2s16(data, byteptr);
                    byteptr += 2;
                }

                m += 1;
            }
        }
    }

    (byteptr == endptr) as i32
}

// Read the shaping weights from specified metadata block into the
// WavpackStream structure. Note that there must be two values (even
// for mono streams) and that the values are stored in the same
// manner as decorrelation weights. These would normally be read from
// the "correction" file and are used for lossless reconstruction of
// hybrid data.

/// Translation of `read_shaping_info()`.
pub(crate) fn read_shaping_info(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    let data = wpmd.bytes();
    let mono = (wps.wphdr.flags & MONO_DATA) != 0;

    if wpmd.byte_length == 2 {
        wps.dc.shaping_acc[0] = ((restore_weight(byte_at(data, 0) as i8) as u32) << 16) as i32;
        wps.dc.shaping_acc[1] = ((restore_weight(byte_at(data, 1) as i8) as u32) << 16) as i32;
        TRUE
    } else if wpmd.byte_length >= if mono { 4 } else { 8 } {
        let mut byteptr = 0usize;

        wps.dc.error[0] = exp2s16(data, byteptr);
        wps.dc.shaping_acc[0] = exp2s16(data, byteptr + 2);
        byteptr += 4;

        if !mono {
            wps.dc.error[1] = exp2s16(data, byteptr);
            wps.dc.shaping_acc[1] = exp2s16(data, byteptr + 2);
            byteptr += 4;
        }

        if wpmd.byte_length == if mono { 6 } else { 12 } {
            wps.dc.shaping_delta[0] = exp2s16(data, byteptr);

            if !mono {
                wps.dc.shaping_delta[1] = exp2s16(data, byteptr + 2);
            }
        }

        TRUE
    } else {
        FALSE
    }
}
