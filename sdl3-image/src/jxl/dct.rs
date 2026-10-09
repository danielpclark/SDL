// Rust translation of lib/jxl/dct_block-inl.h, lib/jxl/transpose-inl.h and
// lib/jxl/dct-inl.h from libjxl (https://github.com/libjxl/libjxl, at the
// revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches), for Highway's scalar target (one lane per vector, so every
// `SZ` is 1 and the transposes are the generic ones).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Fast SIMD floating-point (I)DCT, any power of two.

use super::dct_scales::{wc_multipliers, K_SQRT2};

// ---- dct_block-inl.h ----
// Adapters for DCT input/output: from/to contiguous blocks or image rows.

// Block: (x, y) <-> (N * y + x)
// Lines: (x, y) <-> (stride * y + x)
//
// I.e. Block is a specialization of Lines with fixed stride.
//
// FromXXX should implement Read and Load (Read vector).
// ToXXX should implement Write and Store (Write vector).

/// Translation of `DCTFrom` (over the slice that starts at its `data_`).
#[derive(Clone, Copy)]
pub(crate) struct DctFrom<'a> {
    stride: usize,
    data: &'a [f32],
}

impl<'a> DctFrom<'a> {
    pub(crate) fn new(data: &'a [f32], stride: usize) -> Self {
        DctFrom { stride, data }
    }

    #[inline(always)]
    pub(crate) fn read(&self, row: usize, i: usize) -> f32 {
        self.data[row * self.stride + i]
    }
}

/// Translation of `DCTTo` (over the slice that starts at its `data_`).
pub(crate) struct DctTo<'a> {
    stride: usize,
    data: &'a mut [f32],
}

impl<'a> DctTo<'a> {
    pub(crate) fn new(data: &'a mut [f32], stride: usize) -> Self {
        DctTo { stride, data }
    }

    #[inline(always)]
    pub(crate) fn write(&mut self, v: f32, row: usize, i: usize) {
        self.data[row * self.stride + i] = v;
    }
}

// ---- transpose-inl.h ----
// Block transpose for DCT/IDCT

/// Translation of `Transpose<N, M>::Run()` (`GenericTransposeBlock()`
/// without SIMD, as on the scalar target).
#[inline]
fn transpose(rows: usize, cols: usize, from: &DctFrom<'_>, to: &mut DctTo<'_>) {
    for n in 0..rows {
        for m in 0..cols {
            to.write(from.read(n, m), m, n);
        }
    }
}

// ---- dct-inl.h ----

// Implementation of Lowest Complexity Self Recursive Radix-2 DCT II/III
// Algorithms, by Siriani M. Perera and Jianhua Liu.

// (CoeffBundle<N, 1>: the bundles are contiguous here, as SZ is 1.)

/// `CoeffBundle<N, 1>::B()`.
#[inline(always)]
fn coeff_bundle_b(n: usize, coeff: &mut [f32]) {
    let in1 = coeff[0];
    let in2 = coeff[1];
    coeff[0] = in1 * K_SQRT2 + in2;
    let mut i = 1;
    while i + 1 < n {
        coeff[i] += coeff[i + 1];
        i += 1;
    }
}

/// `CoeffBundle<N, 1>::BTranspose()`.
#[inline(always)]
fn coeff_bundle_b_transpose(n: usize, coeff: &mut [f32]) {
    for i in (1..n).rev() {
        coeff[i] += coeff[i - 1];
    }
    coeff[0] *= K_SQRT2;
}

/// `CoeffBundle<N, 1>::MultiplyAndAdd()`, storing through `store`.
#[inline(always)]
fn coeff_bundle_multiply_and_add(n: usize, coeff: &[f32], mut store: impl FnMut(f32, usize)) {
    let mults = wc_multipliers(n);
    for i in 0..n / 2 {
        let mul = mults[i];
        let in1 = coeff[i];
        let in2 = coeff[n / 2 + i];
        let out1 = mul * in2 + in1;
        let out2 = in1 - mul * in2;
        store(out1, i);
        store(out2, n - i - 1);
    }
}

/// `DCT1DImpl<N, 1>()(mem)`.
fn dct1d_impl(n: usize, mem: &mut [f32]) {
    match n {
        1 => {}
        2 => {
            let in1 = mem[0];
            let in2 = mem[1];
            mem[0] = in1 + in2;
            mem[1] = in1 - in2;
        }
        4 => dct1d_impl_sized::<4>(mem),
        8 => dct1d_impl_sized::<8>(mem),
        16 => dct1d_impl_sized::<16>(mem),
        32 => dct1d_impl_sized::<32>(mem),
        64 => dct1d_impl_sized::<64>(mem),
        128 => dct1d_impl_sized::<128>(mem),
        256 => dct1d_impl_sized::<256>(mem),
        _ => unreachable!("no DCT1DImpl<{n}>"),
    }
}

fn dct1d_impl_sized<const N: usize>(mem: &mut [f32]) {
    // This is relatively small (4kB with 64-DCT and AVX-512)
    let mut tmp = [0f32; N];
    let h = N / 2;
    // CoeffBundle<N / 2, SZ>::AddReverse(mem, mem + N / 2 * SZ, tmp);
    for i in 0..h {
        tmp[i] = mem[i] + mem[h + (h - i - 1)];
    }
    dct1d_impl(h, &mut tmp[..h]);
    // CoeffBundle<N / 2, SZ>::SubReverse(mem, mem + N / 2 * SZ, tmp + N / 2 * SZ);
    for i in 0..h {
        tmp[h + i] = mem[i] - mem[h + (h - i - 1)];
    }
    // CoeffBundle<N, SZ>::Multiply(tmp);
    let mults = wc_multipliers(N);
    for i in 0..h {
        tmp[h + i] *= mults[i];
    }
    dct1d_impl(h, &mut tmp[h..]);
    coeff_bundle_b(h, &mut tmp[h..]);
    // CoeffBundle<N, SZ>::InverseEvenOdd(tmp, mem);
    for i in 0..h {
        mem[2 * i] = tmp[i];
    }
    for i in h..N {
        mem[2 * (i - h) + 1] = tmp[i];
    }
}

/// `IDCT1DImpl<N, 1>()(mem, 1, mem, 1)`: the in-place transform of a
/// contiguous bundle.
fn idct1d_impl_inplace(n: usize, mem: &mut [f32]) {
    match n {
        1 => {}
        2 => {
            let in1 = mem[0];
            let in2 = mem[1];
            mem[0] = in1 + in2;
            mem[1] = in1 - in2;
        }
        4 => idct1d_impl_inplace_sized::<4>(mem),
        8 => idct1d_impl_inplace_sized::<8>(mem),
        16 => idct1d_impl_inplace_sized::<16>(mem),
        32 => idct1d_impl_inplace_sized::<32>(mem),
        64 => idct1d_impl_inplace_sized::<64>(mem),
        128 => idct1d_impl_inplace_sized::<128>(mem),
        256 => idct1d_impl_inplace_sized::<256>(mem),
        _ => unreachable!("no IDCT1DImpl<{n}>"),
    }
}

/// The part of `IDCT1DImpl<N, 1>` between `ForwardEvenOdd()` and
/// `MultiplyAndAdd()`.
#[inline(always)]
fn idct1d_impl_middle(n: usize, tmp: &mut [f32]) {
    let h = n / 2;
    idct1d_impl_inplace(h, &mut tmp[..h]);
    coeff_bundle_b_transpose(h, &mut tmp[h..]);
    idct1d_impl_inplace(h, &mut tmp[h..n]);
}

fn idct1d_impl_inplace_sized<const N: usize>(mem: &mut [f32]) {
    // This is relatively small (4kB with 64-DCT and AVX-512)
    let mut tmp = [0f32; N];
    // CoeffBundle<N, SZ>::ForwardEvenOdd(from, from_stride, tmp);
    for i in 0..N / 2 {
        tmp[i] = mem[2 * i];
    }
    for i in N / 2..N {
        tmp[i] = mem[2 * (i - N / 2) + 1];
    }
    idct1d_impl_middle(N, &mut tmp);
    coeff_bundle_multiply_and_add(N, &tmp, |v, k| mem[k] = v);
}

/// `IDCT1DImpl<N, 1>()(from.Address(0, i), from.Stride(), to.Address(0, j),
/// to.Stride())`: column `i` of `from` to column `j` of `to`.
fn idct1d_impl_strided(n: usize, from: &DctFrom<'_>, i: usize, to: &mut DctTo<'_>, j: usize) {
    match n {
        1 => to.write(from.read(0, i), 0, j),
        2 => {
            let in1 = from.read(0, i);
            let in2 = from.read(1, i);
            to.write(in1 + in2, 0, j);
            to.write(in1 - in2, 1, j);
        }
        4 => idct1d_impl_strided_sized::<4>(from, i, to, j),
        8 => idct1d_impl_strided_sized::<8>(from, i, to, j),
        16 => idct1d_impl_strided_sized::<16>(from, i, to, j),
        32 => idct1d_impl_strided_sized::<32>(from, i, to, j),
        64 => idct1d_impl_strided_sized::<64>(from, i, to, j),
        128 => idct1d_impl_strided_sized::<128>(from, i, to, j),
        256 => idct1d_impl_strided_sized::<256>(from, i, to, j),
        _ => unreachable!("no IDCT1DImpl<{n}>"),
    }
}

fn idct1d_impl_strided_sized<const N: usize>(from: &DctFrom<'_>, i: usize, to: &mut DctTo<'_>, j: usize) {
    // This is relatively small (4kB with 64-DCT and AVX-512)
    let mut tmp = [0f32; N];
    // CoeffBundle<N, SZ>::ForwardEvenOdd(from, from_stride, tmp);
    for k in 0..N / 2 {
        tmp[k] = from.read(2 * k, i);
    }
    for k in N / 2..N {
        tmp[k] = from.read(2 * (k - N / 2) + 1, i);
    }
    idct1d_impl_middle(N, &mut tmp);
    coeff_bundle_multiply_and_add(N, &tmp, |v, k| to.write(v, k, j));
}

/// Translation of `DCT1D<N, M>()(from, to)` (`DCT1DWrapper()`).
fn dct1d(n: usize, m: usize, from: &DctFrom<'_>, to: &mut DctTo<'_>) {
    let mut tmp = [0f32; 256];
    let mul = 1.0f32 / n as f32;
    for i in 0..m {
        // TODO(veluca): consider removing the temporary memory here (as is done in
        // IDCT), if it turns out that some compilers don't optimize away the loads
        // and this is performance-critical.
        // CoeffBundle<N, SZ>::LoadFromBlock(from, i, tmp);
        for k in 0..n {
            tmp[k] = from.read(k, i);
        }
        dct1d_impl(n, &mut tmp[..n]);
        // CoeffBundle<N, SZ>::StoreToBlockAndScale(tmp, to, i);
        for k in 0..n {
            to.write(mul * tmp[k], k, i);
        }
    }
}

/// Translation of `IDCT1D<N, M>()(from, to)` (`IDCT1DWrapper()`).
fn idct1d(n: usize, m: usize, from: &DctFrom<'_>, to: &mut DctTo<'_>) {
    for i in 0..m {
        idct1d_impl_strided(n, from, i, to, i);
    }
}

/// Computes the maybe-transposed, scaled DCT of a block, that needs to be
/// HWY_ALIGN'ed. Translation of `ComputeScaledDCT<ROWS, COLS>()`.
// scratch_space must be aligned, and should have space for ROWS*COLS
// floats.
pub(crate) fn compute_scaled_dct(
    rows: usize,
    cols: usize,
    from: &DctFrom<'_>,
    to: &mut [f32],
    scratch_space: &mut [f32],
) {
    let block = scratch_space;
    if rows < cols {
        dct1d(rows, cols, from, &mut DctTo::new(block, cols));
        transpose(rows, cols, &DctFrom::new(block, cols), &mut DctTo::new(to, rows));
        dct1d(cols, rows, &DctFrom::new(to, rows), &mut DctTo::new(block, rows));
        transpose(cols, rows, &DctFrom::new(block, rows), &mut DctTo::new(to, cols));
    } else {
        dct1d(rows, cols, from, &mut DctTo::new(to, cols));
        transpose(rows, cols, &DctFrom::new(to, cols), &mut DctTo::new(block, rows));
        dct1d(cols, rows, &DctFrom::new(block, rows), &mut DctTo::new(to, rows));
    }
}

/// Computes the maybe-transposed, scaled IDCT of a block, that needs to be
/// HWY_ALIGN'ed. Translation of `ComputeScaledIDCT<ROWS, COLS>()`.
// scratch_space must be aligned, and should have space for ROWS*COLS
// floats.
pub(crate) fn compute_scaled_idct(
    rows: usize,
    cols: usize,
    from: &mut [f32],
    to: &mut DctTo<'_>,
    scratch_space: &mut [f32],
) {
    let block = scratch_space;
    // Reverse the steps done in ComputeScaledDCT.
    if rows < cols {
        transpose(rows, cols, &DctFrom::new(from, cols), &mut DctTo::new(block, rows));
        idct1d(cols, rows, &DctFrom::new(block, rows), &mut DctTo::new(from, rows));
        transpose(cols, rows, &DctFrom::new(from, rows), &mut DctTo::new(block, cols));
        idct1d(rows, cols, &DctFrom::new(block, cols), to);
    } else {
        idct1d(cols, rows, &DctFrom::new(from, rows), &mut DctTo::new(block, rows));
        transpose(cols, rows, &DctFrom::new(block, rows), &mut DctTo::new(from, cols));
        idct1d(rows, cols, &DctFrom::new(from, cols), to);
    }
}
