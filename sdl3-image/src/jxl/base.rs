// Rust translation of lib/jxl/base/status.h, lib/jxl/base/bits.h,
// lib/jxl/base/byte_order.h, lib/jxl/base/padded_bytes.h, lib/jxl/common.h
// and lib/jxl/linalg.h from libjxl (https://github.com/libjxl/libjxl, at the
// revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The status type, bit and byte helpers, shared constants and the small
//! matrix routines of the decoder.

// --- status.h ---

/// Translation of `StatusCode`: the non-fatal "not enough bytes" and the
/// fatal generic error (`kOk` is the `Ok` of [`Status`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StatusCode {
    // Non-fatal errors (negative values).
    NotEnoughBytes,

    // Fatal-errors (positive values)
    GenericError,
}

impl StatusCode {
    /// Returns whether the status code is a fatal error. Translation of
    /// `Status::IsFatalError()`.
    #[allow(dead_code)]
    pub(crate) fn is_fatal_error(self) -> bool {
        self == StatusCode::GenericError
    }
}

/// Translation of `Status`: `Ok(())` is `kOk`.
pub(crate) type Status = Result<(), StatusCode>;

/// A `jxl::Status` value from a `StatusCode` which prints a debug message
/// when enabled. Translation of `JXL_FAILURE()` (the messages are not
/// printed, as `JXL_DEBUG_ON_ERROR` is off in SDL_image's build).
macro_rules! jxl_failure {
    ($($msg:tt)*) => {
        Err($crate::jxl::base::StatusCode::GenericError)
    };
}
pub(crate) use jxl_failure;

/// Translation of `JXL_STATUS(code, ...)`.
macro_rules! jxl_status {
    ($code:expr, $($msg:tt)*) => {
        Err($code)
    };
}
pub(crate) use jxl_status;

/// Returns from the current function with the status if it is not `true`
/// (a `bool` condition). Translation of `JXL_RETURN_IF_ERROR()` on a `bool`.
macro_rules! jxl_return_if_error_bool {
    ($cond:expr) => {
        if !($cond) {
            return Err($crate::jxl::base::StatusCode::GenericError);
        }
    };
}
pub(crate) use jxl_return_if_error_bool;

/// Translation of `bool(status)` for a `Status` that was returned as a bool.
pub(crate) fn status_from_bool(ok: bool) -> Status {
    if ok {
        Ok(())
    } else {
        Err(StatusCode::GenericError)
    }
}

// --- bits.h ---

/// Translation of `Num0BitsAboveMS1Bit_Nonzero()` (32-bit).
#[inline]
pub(crate) fn num0_bits_above_ms1_bit_nonzero_u32(x: u32) -> usize {
    debug_assert!(x != 0);
    x.leading_zeros() as usize
}

/// Translation of `Num0BitsAboveMS1Bit_Nonzero()` (64-bit).
#[inline]
pub(crate) fn num0_bits_above_ms1_bit_nonzero_u64(x: u64) -> usize {
    debug_assert!(x != 0);
    x.leading_zeros() as usize
}

/// Translation of `Num0BitsBelowLS1Bit_Nonzero()` (32-bit).
#[inline]
pub(crate) fn num0_bits_below_ls1_bit_nonzero_u32(x: u32) -> usize {
    debug_assert!(x != 0);
    x.trailing_zeros() as usize
}

/// Translation of `Num0BitsBelowLS1Bit_Nonzero()` (64-bit).
#[inline]
pub(crate) fn num0_bits_below_ls1_bit_nonzero_u64(x: u64) -> usize {
    debug_assert!(x != 0);
    x.trailing_zeros() as usize
}

/// Returns base-2 logarithm, rounded down. Translation of
/// `FloorLog2Nonzero()` (32-bit).
#[inline]
pub(crate) fn floor_log2_nonzero_u32(x: u32) -> usize {
    31 ^ num0_bits_above_ms1_bit_nonzero_u32(x)
}

/// Translation of `FloorLog2Nonzero()` (64-bit, `size_t`).
#[inline]
pub(crate) fn floor_log2_nonzero_u64(x: u64) -> usize {
    63 ^ num0_bits_above_ms1_bit_nonzero_u64(x)
}

/// Returns base-2 logarithm, rounded up. Translation of `CeilLog2Nonzero()`
/// (32-bit).
#[inline]
pub(crate) fn ceil_log2_nonzero_u32(x: u32) -> usize {
    let floor_log2 = floor_log2_nonzero_u32(x);
    if (x & (x - 1)) == 0 {
        return floor_log2; // power of two
    }
    floor_log2 + 1
}

/// Translation of `CeilLog2Nonzero()` (64-bit, `size_t`).
#[inline]
pub(crate) fn ceil_log2_nonzero_u64(x: u64) -> usize {
    let floor_log2 = floor_log2_nonzero_u64(x);
    if (x & (x - 1)) == 0 {
        return floor_log2; // power of two
    }
    floor_log2 + 1
}

// --- byte_order.h ---

/// Translation of `LoadBE16()`.
#[inline]
#[allow(dead_code)]
pub(crate) fn load_be16(p: &[u8]) -> u32 {
    ((p[0] as u32) << 8) | p[1] as u32
}

/// Translation of `LoadLE16()`.
#[inline]
pub(crate) fn load_le16(p: &[u8]) -> u32 {
    ((p[1] as u32) << 8) | p[0] as u32
}

/// Translation of `LoadBE32()`.
#[inline]
pub(crate) fn load_be32(p: &[u8]) -> u32 {
    u32::from_be_bytes([p[0], p[1], p[2], p[3]])
}

/// Translation of `LoadBE64()`.
#[inline]
pub(crate) fn load_be64(p: &[u8]) -> u64 {
    u64::from_be_bytes([p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]])
}

/// Translation of `LoadLE32()`.
#[inline]
#[allow(dead_code)]
pub(crate) fn load_le32(p: &[u8]) -> u32 {
    u32::from_le_bytes([p[0], p[1], p[2], p[3]])
}

/// Translation of `LoadLE64()`.
#[inline]
pub(crate) fn load_le64(p: &[u8]) -> u64 {
    u64::from_le_bytes([p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]])
}

// --- padded_bytes.h ---

/// Translation of `PaddedBytes` (a byte vector; the padding is for the
/// encoder's bit writer).
pub(crate) type PaddedBytes = Vec<u8>;

// --- common.h ---

// Some enums and typedefs used by more than one header file.

pub(crate) const K_BITS_PER_BYTE: usize = 8; // more clear than CHAR_BIT

/// Translation of `RoundUpToBlockDim()`.
#[inline]
#[allow(dead_code)]
pub(crate) fn round_up_to_block_dim(dim: usize) -> usize {
    (dim + 7) & !7usize
}

/// Translation of `SafeAdd()`.
#[inline]
pub(crate) fn safe_add(a: u64, b: u64, sum: &mut u64) -> bool {
    *sum = a.wrapping_add(b);
    *sum >= a // no need to check b - either sum >= both or < both.
}

/// Translation of `DivCeil()` (on `size_t`).
#[inline]
pub(crate) fn div_ceil(a: usize, b: usize) -> usize {
    a.wrapping_add(b).wrapping_sub(1) / b
}

/// Works for any `align`; if a power of two, compiler emits ADD+AND.
/// Translation of `RoundUpTo()`.
#[inline]
pub(crate) fn round_up_to(what: usize, align: usize) -> usize {
    div_ceil(what, align).wrapping_mul(align)
}

pub(crate) const K_PI: f64 = 3.14159265358979323846264338327950288;

// Reasonable default for sRGB, matches common monitors. We map white to this
// many nits (cd/m^2) by default. Butteraugli was tuned for 250 nits, which is
// very close.
pub(crate) const K_DEFAULT_INTENSITY_TARGET: f32 = 255.0;

// Block is the square grid of pixels to which an "energy compaction"
// transformation (e.g. DCT) is applied. Each block has its own AC quantizer.
pub(crate) const K_BLOCK_DIM: usize = 8;

pub(crate) const K_DCT_BLOCK_SIZE: usize = K_BLOCK_DIM * K_BLOCK_DIM;

pub(crate) const K_GROUP_DIM: usize = 256;
pub(crate) const K_GROUP_DIM_IN_BLOCKS: usize = K_GROUP_DIM / K_BLOCK_DIM;

// Maximum number of passes in an image.
pub(crate) const K_MAX_NUM_PASSES: usize = 11;

// Maximum number of reference frames.
pub(crate) const K_MAX_NUM_REFERENCE_FRAMES: usize = 4;

/// Dimensions of a frame, in pixels, and other derived dimensions.
/// Computed from FrameHeader. Translation of `FrameDimensions`.
// TODO(veluca): add extra channels.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FrameDimensions {
    // Image size without any upsampling, i.e. original_size / upsampling.
    pub xsize: usize,
    pub ysize: usize,
    // Original image size.
    pub xsize_upsampled: usize,
    pub ysize_upsampled: usize,
    // Image size after upsampling the padded image.
    pub xsize_upsampled_padded: usize,
    pub ysize_upsampled_padded: usize,
    // Image size after padding to a multiple of kBlockDim (if VarDCT mode).
    pub xsize_padded: usize,
    pub ysize_padded: usize,
    // Image size in kBlockDim blocks.
    pub xsize_blocks: usize,
    pub ysize_blocks: usize,
    // Image size in number of groups.
    pub xsize_groups: usize,
    pub ysize_groups: usize,
    // Image size in number of DC groups.
    pub xsize_dc_groups: usize,
    pub ysize_dc_groups: usize,
    // Number of AC or DC groups.
    pub num_groups: usize,
    pub num_dc_groups: usize,
    // Size of a group.
    pub group_dim: usize,
    pub dc_group_dim: usize,
}

impl FrameDimensions {
    /// Translation of `FrameDimensions::Set()`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set(
        &mut self,
        xsize: usize,
        ysize: usize,
        group_size_shift: usize,
        max_hshift: usize,
        max_vshift: usize,
        modular_mode: bool,
        upsampling: usize,
    ) {
        self.group_dim = (K_GROUP_DIM >> 1) << group_size_shift;
        self.dc_group_dim = self.group_dim * K_BLOCK_DIM;
        self.xsize_upsampled = xsize;
        self.ysize_upsampled = ysize;
        self.xsize = div_ceil(xsize, upsampling);
        self.ysize = div_ceil(ysize, upsampling);
        self.xsize_blocks = div_ceil(self.xsize, K_BLOCK_DIM << max_hshift) << max_hshift;
        self.ysize_blocks = div_ceil(self.ysize, K_BLOCK_DIM << max_vshift) << max_vshift;
        self.xsize_padded = self.xsize_blocks * K_BLOCK_DIM;
        self.ysize_padded = self.ysize_blocks * K_BLOCK_DIM;
        if modular_mode {
            // Modular mode doesn't have any padding.
            self.xsize_padded = self.xsize;
            self.ysize_padded = self.ysize;
        }
        self.xsize_upsampled_padded = self.xsize_padded * upsampling;
        self.ysize_upsampled_padded = self.ysize_padded * upsampling;
        self.xsize_groups = div_ceil(self.xsize, self.group_dim);
        self.ysize_groups = div_ceil(self.ysize, self.group_dim);
        self.xsize_dc_groups = div_ceil(self.xsize_blocks, self.group_dim);
        self.ysize_dc_groups = div_ceil(self.ysize_blocks, self.group_dim);
        self.num_groups = self.xsize_groups * self.ysize_groups;
        self.num_dc_groups = self.xsize_dc_groups * self.ysize_dc_groups;
    }
}

/// Translation of `Clamp1()`.
#[inline]
pub(crate) fn clamp1<T: PartialOrd>(val: T, low: T, hi: T) -> T {
    if val < low {
        low
    } else if val > hi {
        hi
    } else {
        val
    }
}

/// Encodes non-negative (X) into (2 * X), negative (-X) into (2 * X - 1).
/// Translation of `PackSigned()`.
#[inline]
pub(crate) fn pack_signed(value: i32) -> u32 {
    ((value as u32) << 1) ^ (((!value) as u32 >> 31).wrapping_sub(1))
}

/// Reverse to PackSigned, i.e. UnpackSigned(PackSigned(X)) == X.
/// (((~value) & 1) - 1) is either 0 or 0xFF...FF and it will have an
/// expected unsigned-integer-overflow. Translation of `UnpackSigned()`.
#[inline]
pub(crate) fn unpack_signed(value: usize) -> isize {
    ((value >> 1) ^ ((!value) & 1).wrapping_sub(1)) as isize
}

/// Translation of `DecodeVarInt()`.
#[allow(dead_code)]
pub(crate) fn decode_var_int(input: &[u8], input_size: usize, pos: &mut usize) -> u64 {
    let mut ret: u64 = 0;
    let mut i = 0;
    while *pos + i < input_size && i < 10 {
        ret |= ((input[*pos + i] & 127) as u64) << (7 * i as u64);
        // If the next-byte flag is not set, stop
        if (input[*pos + i] & 128) == 0 {
            break;
        }
        i += 1;
    }
    // TODO: Return a decoding error if i == 10.
    *pos += i + 1;
    ret
}

// --- linalg.h ---

/// Computes A = B * C, with sizes rows*cols: A=ha*wa, B=wa*wb, C=ha*wb.
/// Translation of `MatMul()` (on floats: the products are float, the sums
/// double).
pub(crate) fn mat_mul(a: &[f32], b: &[f32], ha: usize, wa: usize, wb: usize, c: &mut [f32]) {
    let mut temp = vec![0f32; wa]; // Make better use of cache lines
    for x in 0..wb {
        for z in 0..wa {
            temp[z] = b[z * wb + x];
        }
        for y in 0..ha {
            let mut e: f64 = 0.0;
            for z in 0..wa {
                e += (a[y * wa + z] * temp[z]) as f64;
            }
            c[y * wb + x] = e as f32;
        }
    }
}

/// Translation of `Inv3x3Matrix()` (on floats).
pub(crate) fn inv3x3_matrix(matrix: &mut [f32; 9]) -> Status {
    // Intermediate computation is done in double precision.
    let m = |i: usize| matrix[i] as f64;
    let mut temp = [0f64; 9];
    temp[0] = m(4) * m(8) - m(5) * m(7);
    temp[1] = m(2) * m(7) - m(1) * m(8);
    temp[2] = m(1) * m(5) - m(2) * m(4);
    temp[3] = m(5) * m(6) - m(3) * m(8);
    temp[4] = m(0) * m(8) - m(2) * m(6);
    temp[5] = m(2) * m(3) - m(0) * m(5);
    temp[6] = m(3) * m(7) - m(4) * m(6);
    temp[7] = m(1) * m(6) - m(0) * m(7);
    temp[8] = m(0) * m(4) - m(1) * m(3);
    let det = m(0) * temp[0] + m(1) * temp[3] + m(2) * temp[6];
    if det.abs() < 1e-10 {
        return jxl_failure!("Matrix determinant is too close to 0");
    }
    let idet = 1.0 / det;
    for i in 0..9 {
        matrix[i] = (temp[i] * idet) as f32;
    }
    Ok(())
}
