// Rust translation of src/enc/iterator_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! VP8Iterator: block iterator. The iterator is passed alongside the
//! encoder it walks (`it->enc_` upstream). The progress report and
//! `VP8IteratorExport()` (for `show_compressed`, which SDL_image doesn't
//! set) are not translated.

use crate::webp::dsp::BPS;
use crate::webp::enc::quant_enc::VP8_SCAN;
use crate::webp::enc::vp8i_enc::{
    VP8EncIterator, VP8Encoder, U_LEFT, U_OFF_ENC, V_LEFT, V_OFF_ENC, Y_LEFT, Y_OFF_ENC,
};

//------------------------------------------------------------------------------
// VP8Iterator
//------------------------------------------------------------------------------

/// Translation of `InitLeft()`.
fn init_left(it: &mut VP8EncIterator) {
    let v = if it.y > 0 { 129 } else { 127 };
    it.yuv_left_mem[Y_LEFT - 1] = v;
    it.yuv_left_mem[U_LEFT - 1] = v;
    it.yuv_left_mem[V_LEFT - 1] = v;
    it.yuv_left_mem[Y_LEFT..Y_LEFT + 16].fill(129);
    it.yuv_left_mem[U_LEFT..U_LEFT + 8].fill(129);
    it.yuv_left_mem[V_LEFT..V_LEFT + 8].fill(129);
    it.left_nz[8] = 0;
    if it.has_top_derr {
        it.left_derr = [[0; 2]; 2];
    }
}

/// Translation of `InitTop()`.
fn init_top(enc: &mut VP8Encoder<'_>) {
    let top_size = enc.mb_w as usize * 16;
    enc.y_top[..top_size].fill(127);
    enc.uv_top[..top_size].fill(127);
    enc.nz[1..1 + enc.mb_w as usize].fill(0);
    if !enc.top_derr.is_empty() {
        enc.top_derr.fill([[0; 2]; 2]);
    }
}

/// reset iterator position to row 'y'. Translation of `VP8IteratorSetRow()`.
pub(crate) fn vp8_iterator_set_row(it: &mut VP8EncIterator, enc: &VP8Encoder<'_>, y: i32) {
    it.x = 0;
    it.y = y;
    it.bw = (y & (enc.num_parts - 1)) as usize;
    it.preds = (1 + enc.preds_w + y * 4 * enc.preds_w) as usize;
    it.nz = 1;
    it.mb = (y * enc.mb_w) as usize;
    it.y_top = 0;
    it.uv_top = 0;
    it.use_tmp_32 = false;
    init_left(it);
}

/// restart a scan. Translation of `VP8IteratorReset()`.
pub(crate) fn vp8_iterator_reset(it: &mut VP8EncIterator, enc: &mut VP8Encoder<'_>) {
    vp8_iterator_set_row(it, enc, 0);
    vp8_iterator_set_count_down(it, enc.mb_w * enc.mb_h); // default
    init_top(enc);
    it.do_trellis = false;
}

/// set count down (=number of iterations to go).
/// Translation of `VP8IteratorSetCountDown()`.
pub(crate) fn vp8_iterator_set_count_down(it: &mut VP8EncIterator, count_down: i32) {
    it.count_down = count_down;
    it.count_down0 = count_down;
}

/// return true if iteration is finished. Translation of
/// `VP8IteratorIsDone()`.
pub(crate) fn vp8_iterator_is_done(it: &VP8EncIterator) -> bool {
    it.count_down <= 0
}

/// must be called first. Translation of `VP8IteratorInit()`.
pub(crate) fn vp8_iterator_init(enc: &mut VP8Encoder<'_>) -> VP8EncIterator {
    let mut it = VP8EncIterator {
        has_top_derr: !enc.top_derr.is_empty(),
        ..Default::default()
    };
    vp8_iterator_reset(&mut it, enc);
    it
}

//------------------------------------------------------------------------------
// Import the source samples into the cache. Takes care of replicating
// boundary pixels if necessary.

/// Translation of `MinSize()`.
fn min_size(a: i32, b: i32) -> i32 {
    if a < b {
        a
    } else {
        b
    }
}

/// Translation of `ImportBlock()`.
fn import_block(src: &[u8], src_stride: usize, dst: &mut [u8], w: usize, h: usize, size: usize) {
    for i in 0..h {
        let d = i * BPS;
        dst[d..d + w].copy_from_slice(&src[i * src_stride..i * src_stride + w]);
        if w < size {
            let v = dst[d + w - 1];
            dst[d + w..d + size].fill(v);
        }
    }
    for i in h..size {
        dst.copy_within((i - 1) * BPS..(i - 1) * BPS + size, i * BPS);
    }
}

/// Translation of `ImportLine()`: `len` samples `src_stride` apart from
/// `src[off]`.
fn import_line(
    src: &[u8],
    off: usize,
    src_stride: usize,
    dst: &mut [u8],
    len: usize,
    total_len: usize,
) {
    for i in 0..len {
        dst[i] = src[off + i * src_stride];
    }
    for i in len..total_len {
        dst[i] = dst[len - 1];
    }
}

/// Import uncompressed samples from source.
/// If tmp_32 is true, import boundary samples too (into the iterator's
/// `tmp_32`). Translation of `VP8IteratorImport()`.
pub(crate) fn vp8_iterator_import(it: &mut VP8EncIterator, enc: &VP8Encoder<'_>, tmp_32: bool) {
    let x = it.x;
    let y = it.y;
    let pic = enc.pic;
    let y_stride = pic.y_stride as usize;
    let uv_stride = pic.uv_stride as usize;
    let ysrc = (y as usize * y_stride + x as usize) * 16;
    let usrc = (y as usize * uv_stride + x as usize) * 8;
    let vsrc = usrc;
    let w = min_size(pic.width - x * 16, 16) as usize;
    let h = min_size(pic.height - y * 16, 16) as usize;
    let uv_w = (w + 1) >> 1;
    let uv_h = (h + 1) >> 1;

    import_block(
        &pic.y[ysrc..],
        y_stride,
        &mut it.yuv_in[Y_OFF_ENC..],
        w,
        h,
        16,
    );
    import_block(
        &pic.u[usrc..],
        uv_stride,
        &mut it.yuv_in[U_OFF_ENC..],
        uv_w,
        uv_h,
        8,
    );
    import_block(
        &pic.v[vsrc..],
        uv_stride,
        &mut it.yuv_in[V_OFF_ENC..],
        uv_w,
        uv_h,
        8,
    );

    if !tmp_32 {
        return;
    }

    // Import source (uncompressed) samples into boundary.
    if x == 0 {
        init_left(it);
    } else {
        if y == 0 {
            it.yuv_left_mem[Y_LEFT - 1] = 127;
            it.yuv_left_mem[U_LEFT - 1] = 127;
            it.yuv_left_mem[V_LEFT - 1] = 127;
        } else {
            it.yuv_left_mem[Y_LEFT - 1] = pic.y[ysrc - 1 - y_stride];
            it.yuv_left_mem[U_LEFT - 1] = pic.u[usrc - 1 - uv_stride];
            it.yuv_left_mem[V_LEFT - 1] = pic.v[vsrc - 1 - uv_stride];
        }
        import_line(
            &pic.y,
            ysrc - 1,
            y_stride,
            &mut it.yuv_left_mem[Y_LEFT..],
            h,
            16,
        );
        import_line(
            &pic.u,
            usrc - 1,
            uv_stride,
            &mut it.yuv_left_mem[U_LEFT..],
            uv_h,
            8,
        );
        import_line(
            &pic.v,
            vsrc - 1,
            uv_stride,
            &mut it.yuv_left_mem[V_LEFT..],
            uv_h,
            8,
        );
    }

    it.use_tmp_32 = true; // y_top_ = tmp_32 + 0, uv_top_ = tmp_32 + 16
    if y == 0 {
        it.tmp_32.fill(127);
    } else {
        import_line(&pic.y, ysrc - y_stride, 1, &mut it.tmp_32, w, 16);
        import_line(&pic.u, usrc - uv_stride, 1, &mut it.tmp_32[16..], uv_w, 8);
        import_line(
            &pic.v,
            vsrc - uv_stride,
            1,
            &mut it.tmp_32[16 + 8..],
            uv_w,
            8,
        );
    }
}

/// The top luma samples at position 'x_' (`it->y_top_`), up to the end
/// of their buffer.
pub(crate) fn y_top<'a>(it: &'a VP8EncIterator, enc: &'a VP8Encoder<'_>) -> &'a [u8] {
    if it.use_tmp_32 {
        &it.tmp_32
    } else {
        &enc.y_top[it.y_top..]
    }
}

/// The top u/v samples at position 'x_' (`it->uv_top_`), up to the end
/// of their buffer.
pub(crate) fn uv_top<'a>(it: &'a VP8EncIterator, enc: &'a VP8Encoder<'_>) -> &'a [u8] {
    if it.use_tmp_32 {
        &it.tmp_32[16..]
    } else {
        &enc.uv_top[it.uv_top..]
    }
}

//------------------------------------------------------------------------------
// Non-zero contexts setup/teardown

// Nz bits:
//  0  1  2  3  Y
//  4  5  6  7
//  8  9 10 11
// 12 13 14 15
// 16 17        U
// 18 19
// 20 21        V
// 22 23
// 24           DC-intra16

/// Convert packed context to byte array. Translation of the `BIT()` macro.
fn bit(nz: u32, n: u32) -> i32 {
    (nz & (1 << n) != 0) as i32
}

/// Translation of `VP8IteratorNzToBytes()`.
pub(crate) fn vp8_iterator_nz_to_bytes(it: &mut VP8EncIterator, enc: &VP8Encoder<'_>) {
    let tnz = enc.nz[it.nz];
    let lnz = enc.nz[it.nz - 1];
    let top_nz = &mut it.top_nz;
    let left_nz = &mut it.left_nz;

    // Top-Y
    top_nz[0] = bit(tnz, 12);
    top_nz[1] = bit(tnz, 13);
    top_nz[2] = bit(tnz, 14);
    top_nz[3] = bit(tnz, 15);
    // Top-U
    top_nz[4] = bit(tnz, 18);
    top_nz[5] = bit(tnz, 19);
    // Top-V
    top_nz[6] = bit(tnz, 22);
    top_nz[7] = bit(tnz, 23);
    // DC
    top_nz[8] = bit(tnz, 24);

    // left-Y
    left_nz[0] = bit(lnz, 3);
    left_nz[1] = bit(lnz, 7);
    left_nz[2] = bit(lnz, 11);
    left_nz[3] = bit(lnz, 15);
    // left-U
    left_nz[4] = bit(lnz, 17);
    left_nz[5] = bit(lnz, 19);
    // left-V
    left_nz[6] = bit(lnz, 21);
    left_nz[7] = bit(lnz, 23);
    // left-DC is special, iterated separately
}

/// Translation of `VP8IteratorBytesToNz()`.
pub(crate) fn vp8_iterator_bytes_to_nz(it: &VP8EncIterator, enc: &mut VP8Encoder<'_>) {
    let mut nz: u32 = 0;
    let top_nz = it.top_nz.map(|v| v as u32);
    let left_nz = it.left_nz.map(|v| v as u32);
    // top
    nz |= (top_nz[0] << 12) | (top_nz[1] << 13);
    nz |= (top_nz[2] << 14) | (top_nz[3] << 15);
    nz |= (top_nz[4] << 18) | (top_nz[5] << 19);
    nz |= (top_nz[6] << 22) | (top_nz[7] << 23);
    nz |= top_nz[8] << 24; // we propagate the _top_ bit, esp. for intra4
                           // left
    nz |= (left_nz[0] << 3) | (left_nz[1] << 7);
    nz |= left_nz[2] << 11;
    nz |= (left_nz[4] << 17) | (left_nz[6] << 21);

    enc.nz[it.nz] = nz;
}

//------------------------------------------------------------------------------
// Advance to the next position, doing the bookkeeping.

/// save the yuv_out_ boundary values to top_/left_ arrays for next
/// iterations. Translation of `VP8IteratorSaveBoundary()`.
pub(crate) fn vp8_iterator_save_boundary(it: &mut VP8EncIterator, enc: &mut VP8Encoder<'_>) {
    let x = it.x;
    let y = it.y;
    let ysrc = &it.yuv_out[Y_OFF_ENC..];
    let uvsrc = &it.yuv_out[U_OFF_ENC..];
    if x < enc.mb_w - 1 {
        // left
        for i in 0..16 {
            it.yuv_left_mem[Y_LEFT + i] = ysrc[15 + i * BPS];
        }
        for i in 0..8 {
            it.yuv_left_mem[U_LEFT + i] = uvsrc[7 + i * BPS];
            it.yuv_left_mem[V_LEFT + i] = uvsrc[15 + i * BPS];
        }
        // top-left (before 'top'!)
        it.yuv_left_mem[Y_LEFT - 1] = y_top(it, enc)[15];
        it.yuv_left_mem[U_LEFT - 1] = uv_top(it, enc)[7];
        it.yuv_left_mem[V_LEFT - 1] = uv_top(it, enc)[8 + 7];
    }
    if y < enc.mb_h - 1 {
        // top
        debug_assert!(!it.use_tmp_32);
        enc.y_top[it.y_top..it.y_top + 16].copy_from_slice(&ysrc[15 * BPS..15 * BPS + 16]);
        enc.uv_top[it.uv_top..it.uv_top + 16].copy_from_slice(&uvsrc[7 * BPS..7 * BPS + 16]);
    }
}

/// go to next macroblock. Returns false if not finished.
/// Translation of `VP8IteratorNext()`.
pub(crate) fn vp8_iterator_next(it: &mut VP8EncIterator, enc: &VP8Encoder<'_>) -> bool {
    it.x += 1;
    if it.x == enc.mb_w {
        it.y += 1;
        vp8_iterator_set_row(it, enc, it.y);
    } else {
        it.preds += 4;
        it.mb += 1;
        it.nz += 1;
        it.y_top += 16;
        it.uv_top += 16;
    }
    it.count_down -= 1;
    0 < it.count_down
}

//------------------------------------------------------------------------------
// Helper function to set mode properties

/// Translation of `VP8SetIntra16Mode()`.
pub(crate) fn vp8_set_intra16_mode(it: &VP8EncIterator, enc: &mut VP8Encoder<'_>, mode: i32) {
    let mut preds = it.preds;
    for _ in 0..4 {
        enc.preds[preds..preds + 4].fill(mode as u8);
        preds += enc.preds_w as usize;
    }
    enc.mb_info[it.mb].type_ = 1;
}

/// Translation of `VP8SetIntra4Mode()`.
pub(crate) fn vp8_set_intra4_mode(it: &VP8EncIterator, enc: &mut VP8Encoder<'_>, modes: &[u8; 16]) {
    let mut preds = it.preds;
    for y in 0..4 {
        enc.preds[preds..preds + 4].copy_from_slice(&modes[y * 4..y * 4 + 4]);
        preds += enc.preds_w as usize;
    }
    enc.mb_info[it.mb].type_ = 0;
}

/// Translation of `VP8SetIntraUVMode()`.
pub(crate) fn vp8_set_intra_uv_mode(it: &VP8EncIterator, enc: &mut VP8Encoder<'_>, mode: i32) {
    enc.mb_info[it.mb].uv_mode = mode as u8;
}

/// Translation of `VP8SetSkip()`.
pub(crate) fn vp8_set_skip(it: &VP8EncIterator, enc: &mut VP8Encoder<'_>, skip: bool) {
    enc.mb_info[it.mb].skip = skip;
}

/// Translation of `VP8SetSegment()`.
pub(crate) fn vp8_set_segment(it: &VP8EncIterator, enc: &mut VP8Encoder<'_>, segment: i32) {
    enc.mb_info[it.mb].segment = segment as u8;
}

//------------------------------------------------------------------------------
// Intra4x4 sub-blocks iteration
//
//  We store and update the boundary samples into an array of 37 pixels. They
//  are updated as we iterate and reconstructs each intra4x4 blocks in turn.
//  The position of the samples has the following snake pattern:
//
// 16|17 18 19 20|21 22 23 24|25 26 27 28|29 30 31 32|33 34 35 36  <- Top-right
// --+-----------+-----------+-----------+-----------+
// 15|         19|         23|         27|         31|
// 14|         18|         22|         26|         30|
// 13|         17|         21|         25|         29|
// 12|13 14 15 16|17 18 19 20|21 22 23 24|25 26 27 28|
// --+-----------+-----------+-----------+-----------+
// 11|         15|         19|         23|         27|
// 10|         14|         18|         22|         26|
//  9|         13|         17|         21|         25|
//  8| 9 10 11 12|13 14 15 16|17 18 19 20|21 22 23 24|
// --+-----------+-----------+-----------+-----------+
//  7|         11|         15|         19|         23|
//  6|         10|         14|         18|         22|
//  5|          9|         13|         17|         21|
//  4| 5  6  7  8| 9 10 11 12|13 14 15 16|17 18 19 20|
// --+-----------+-----------+-----------+-----------+
//  3|          7|         11|         15|         19|
//  2|          6|         10|         14|         18|
//  1|          5|          9|         13|         17|
//  0| 1  2  3  4| 5  6  7  8| 9 10 11 12|13 14 15 16|
// --+-----------+-----------+-----------+-----------+

/// Array to record the position of the top sample to pass to the prediction
/// functions in dsp.c. Translation of `VP8TopLeftI4`.
const VP8_TOP_LEFT_I4: [u8; 16] = [17, 21, 25, 29, 13, 17, 21, 25, 9, 13, 17, 21, 5, 9, 13, 17];

/// Intra4x4 iterations. Translation of `VP8IteratorStartI4()`.
pub(crate) fn vp8_iterator_start_i4(it: &mut VP8EncIterator, enc: &VP8Encoder<'_>) {
    it.i4 = 0; // first 4x4 sub-block
    it.i4_top = VP8_TOP_LEFT_I4[0] as usize;

    // Import the boundary samples
    for i in 0..17 {
        // left
        it.i4_boundary[i] = it.yuv_left_mem[Y_LEFT + 15 - i];
    }
    let top = y_top(it, enc);
    let mut boundary = it.i4_boundary;
    // top
    boundary[17..17 + 16].copy_from_slice(&top[..16]);
    // top-right samples have a special case on the far right of the picture
    if it.x < enc.mb_w - 1 {
        boundary[17 + 16..17 + 20].copy_from_slice(&top[16..20]);
    } else {
        // else, replicate the last valid pixel four times
        for i in 16..16 + 4 {
            boundary[17 + i] = boundary[17 + 15];
        }
    }
    it.i4_boundary = boundary;
    vp8_iterator_nz_to_bytes(it, enc); // import the non-zero context
}

/// returns true if not done. Translation of `VP8IteratorRotateI4()`:
/// `yuv_out` is `it->yuv_out_` if `use_out` and `it->yuv_out2_`
/// otherwise.
pub(crate) fn vp8_iterator_rotate_i4(it: &mut VP8EncIterator, use_out: bool) -> bool {
    let yuv_out = if use_out { &it.yuv_out } else { &it.yuv_out2 };
    let blk = &yuv_out[Y_OFF_ENC + VP8_SCAN[it.i4 as usize] as usize..];
    let top = it.i4_top;

    // Update the cache with 7 fresh samples
    for i in 0..=3 {
        it.i4_boundary[top - 4 + i] = blk[i + 3 * BPS]; // store future top samples
    }
    if (it.i4 & 3) != 3 {
        // if not on the right sub-blocks #3, #7, #11, #15
        for i in 0..=2 {
            // store future left samples
            it.i4_boundary[top + i] = blk[3 + (2 - i) * BPS];
        }
    } else {
        // else replicate top-right samples, as says the specs.
        for i in 0..=3 {
            it.i4_boundary[top + i] = it.i4_boundary[top + i + 4];
        }
    }
    // move pointers to next sub-block
    it.i4 += 1;
    if it.i4 == 16 {
        // we're done
        return false;
    }

    it.i4_top = VP8_TOP_LEFT_I4[it.i4 as usize] as usize;
    true
}
