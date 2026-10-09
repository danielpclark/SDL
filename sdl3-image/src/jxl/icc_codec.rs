// Rust translation of lib/jxl/icc_codec.h, lib/jxl/icc_codec.cc,
// lib/jxl/icc_codec_common.h and lib/jxl/icc_codec_common.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decoding of the compressed ICC profile. (The decoder only reads a
//! profile in one go: `ICCReader::Init()` and `Process()` are run together;
//! the state that lets upstream resume with more input is not kept.)

use super::base::{decode_var_int, jxl_failure, jxl_status, load_be32, PaddedBytes, Status, StatusCode};
use super::dec_ans::{decode_histograms, AnsCode, AnsSymbolReader, Checkpoint};
use super::dec_bit_reader::BitReader;
use super::fields::u64_coder_read;

// --- icc_codec_common.h ---

const K_ICC_HEADER_SIZE: usize = 128;

type Tag = [u8; 4];

const K_ACSP_TAG: Tag = *b"acsp";
const K_BKPT_TAG: Tag = *b"bkpt";
const K_BTRC_TAG: Tag = *b"bTRC";
const K_BXYZ_TAG: Tag = *b"bXYZ";
const K_CHAD_TAG: Tag = *b"chad";
const K_CHRM_TAG: Tag = *b"chrm";
const K_CPRT_TAG: Tag = *b"cprt";
const K_CURV_TAG: Tag = *b"curv";
const K_DESC_TAG: Tag = *b"desc";
const K_DMDD_TAG: Tag = *b"dmdd";
const K_DMND_TAG: Tag = *b"dmnd";
const K_GBD_TAG: Tag = *b"gbd ";
const K_GTRC_TAG: Tag = *b"gTRC";
const K_GXYZ_TAG: Tag = *b"gXYZ";
const K_KTRC_TAG: Tag = *b"kTRC";
const K_KXYZ_TAG: Tag = *b"kXYZ";
const K_LUMI_TAG: Tag = *b"lumi";
const K_MLUC_TAG: Tag = *b"mluc";
const K_MNTR_TAG: Tag = *b"mntr";
const K_PARA_TAG: Tag = *b"para";
const K_RGB_TAG: Tag = *b"RGB ";
const K_RTRC_TAG: Tag = *b"rTRC";
const K_RXYZ_TAG: Tag = *b"rXYZ";
const K_SF32_TAG: Tag = *b"sf32";
const K_TEXT_TAG: Tag = *b"text";
const K_WTPT_TAG: Tag = *b"wtpt";
const K_XYZ_TAG: Tag = *b"XYZ ";

const K_TAG_STRINGS: [Tag; 17] = [
    K_CPRT_TAG, K_WTPT_TAG, K_BKPT_TAG, K_RXYZ_TAG, K_GXYZ_TAG, K_BXYZ_TAG, K_KXYZ_TAG, K_RTRC_TAG, K_GTRC_TAG,
    K_BTRC_TAG, K_KTRC_TAG, K_CHAD_TAG, K_DESC_TAG, K_CHRM_TAG, K_DMND_TAG, K_DMDD_TAG, K_LUMI_TAG,
];

const K_COMMAND_TAG_UNKNOWN: u8 = 1;
const K_COMMAND_TAG_TRC: u8 = 2;
const K_COMMAND_TAG_XYZ: u8 = 3;
const K_COMMAND_TAG_STRING_FIRST: u8 = 4;

const K_TYPE_STRINGS: [Tag; 8] = [
    K_XYZ_TAG, K_DESC_TAG, K_TEXT_TAG, K_MLUC_TAG, K_PARA_TAG, K_CURV_TAG, K_SF32_TAG, K_GBD_TAG,
];

const K_COMMAND_INSERT: u8 = 1;
const K_COMMAND_SHUFFLE2: u8 = 2;
const K_COMMAND_SHUFFLE4: u8 = 3;
const K_COMMAND_PREDICT: u8 = 4;
const K_COMMAND_XYZ: u8 = 10;
const K_COMMAND_TYPE_START_FIRST: u8 = 16;

const K_FLAG_BIT_OFFSET: u8 = 64;
const K_FLAG_BIT_SIZE: u8 = 128;

const K_NUM_ICC_CONTEXTS: usize = 41;

// --- icc_codec_common.cc ---

fn byte_kind1(b: u8) -> u8 {
    if b.is_ascii_lowercase() {
        return 0;
    }
    if b.is_ascii_uppercase() {
        return 0;
    }
    if b.is_ascii_digit() {
        return 1;
    }
    if b == b'.' || b == b',' {
        return 1;
    }
    if b == 0 {
        return 2;
    }
    if b == 1 {
        return 3;
    }
    if b < 16 {
        return 4;
    }
    if b == 255 {
        return 6;
    }
    if b > 240 {
        return 5;
    }
    7
}

fn byte_kind2(b: u8) -> u8 {
    if b.is_ascii_lowercase() {
        return 0;
    }
    if b.is_ascii_uppercase() {
        return 0;
    }
    if b.is_ascii_digit() {
        return 1;
    }
    if b == b'.' || b == b',' {
        return 1;
    }
    if b < 16 {
        return 2;
    }
    if b > 240 {
        return 3;
    }
    4
}

/// Translation of `PredictValue()` (computed as int, like the promoted C
/// arithmetic, for the 8- and 16-bit types).
fn predict_value_i64(p1: i64, p2: i64, p3: i64, order: i32) -> i64 {
    match order {
        0 => p1,
        1 => 2 * p1 - p2,
        2 => 3 * p1 - 3 * p2 + p3,
        _ => 0,
    }
}

/// Translation of `DecodeUint32()`.
fn decode_uint32(data: &[u8], size: usize, pos: usize) -> u32 {
    if pos.wrapping_add(4) > size {
        0
    } else {
        load_be32(&data[pos..])
    }
}

/// Translation of `EncodeUint32()`.
fn encode_uint32(pos: usize, value: u32, data: &mut PaddedBytes) {
    if pos + 4 > data.len() {
        return;
    }
    data[pos..pos + 4].copy_from_slice(&value.to_be_bytes());
}

/// Translation of `AppendUint32()`.
fn append_uint32(value: u32, data: &mut PaddedBytes) {
    data.resize(data.len() + 4, 0);
    let n = data.len();
    encode_uint32(n - 4, value, data);
}

/// Translation of `DecodeKeyword()`.
fn decode_keyword(data: &[u8], size: usize, pos: usize) -> Tag {
    if pos + 4 > size {
        return *b"    ";
    }
    [data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]
}

/// Translation of `EncodeKeyword()`.
fn encode_keyword(keyword: &Tag, data: &mut [u8], size: usize, pos: usize) {
    if pos + 3 >= size {
        return;
    }
    data[pos..pos + 4].copy_from_slice(keyword);
}

/// Translation of `AppendKeyword()`.
fn append_keyword(keyword: &Tag, data: &mut PaddedBytes) {
    data.extend_from_slice(keyword);
}

// Checks if a + b > size, taking possible integer overflow into account.
/// Translation of `CheckOutOfBounds()`.
fn check_out_of_bounds(a: usize, b: u64, size: usize) -> Status {
    let pos = (a as u64).wrapping_add(b);
    if pos > size as u64 {
        return jxl_failure!("Out of bounds");
    }
    if pos < a as u64 {
        return jxl_failure!("Out of bounds"); // overflow happened
    }
    Ok(())
}

/// Translation of `CheckIs32Bit()`.
fn check_is_32bit(v: u64) -> Status {
    const K_UPPER32: u64 = !0xFFFFFFFFu64;
    if (v & K_UPPER32) != 0 {
        return jxl_failure!("32-bit value expected");
    }
    Ok(())
}

/// Translation of `ICCInitialHeaderPrediction()`.
fn icc_initial_header_prediction() -> PaddedBytes {
    let mut result = vec![0u8; K_ICC_HEADER_SIZE];
    result[8] = 4;
    let n = result.len();
    encode_keyword(&K_MNTR_TAG, &mut result, n, 12);
    encode_keyword(&K_RGB_TAG, &mut result, n, 16);
    encode_keyword(&K_XYZ_TAG, &mut result, n, 20);
    encode_keyword(&K_ACSP_TAG, &mut result, n, 36);
    result[68] = 0;
    result[69] = 0;
    result[70] = 246;
    result[71] = 214;
    result[72] = 0;
    result[73] = 1;
    result[74] = 0;
    result[75] = 0;
    result[76] = 0;
    result[77] = 0;
    result[78] = 211;
    result[79] = 45;
    result
}

/// Translation of `ICCPredictHeader()`.
fn icc_predict_header(icc: &[u8], size: usize, header: &mut [u8], pos: usize) {
    if pos == 8 && size >= 8 {
        header[80] = icc[4];
        header[81] = icc[5];
        header[82] = icc[6];
        header[83] = icc[7];
    }
    if pos == 41 && size >= 41 {
        if icc[40] == b'A' {
            header[41] = b'P';
            header[42] = b'P';
            header[43] = b'L';
        }
        if icc[40] == b'M' {
            header[41] = b'S';
            header[42] = b'F';
            header[43] = b'T';
        }
    }
    if pos == 42 && size >= 42 {
        if icc[40] == b'S' && icc[41] == b'G' {
            header[42] = b'I';
            header[43] = b' ';
        }
        if icc[40] == b'S' && icc[41] == b'U' {
            header[42] = b'N';
            header[43] = b'W';
        }
    }
}

// Predicts a value with linear prediction of given order (0-2), for integers
// with width bytes and given stride in bytes between values.
// The start position is at start + i, and the relevant modulus of i describes
// which byte of the multi-byte integer is being handled.
// The value start + i must be at least stride * 4.
/// Translation of `LinearPredictICCValue()`.
fn linear_predict_icc_value(data: &[u8], start: usize, i: usize, stride: usize, width: usize, order: i32) -> u8 {
    let pos = start + i;
    if width == 1 {
        let p1 = data[pos - stride] as i64;
        let p2 = data[pos - stride * 2] as i64;
        let p3 = data[pos - stride * 3] as i64;
        predict_value_i64(p1, p2, p3, order) as u8
    } else if width == 2 {
        let p = start + (i & !1);
        let p1 = ((data[p - stride] as u16) << 8).wrapping_add(data[p - stride + 1] as u16) as i64;
        let p2 = ((data[p - stride * 2] as u16) << 8).wrapping_add(data[p - stride * 2 + 1] as u16) as i64;
        let p3 = ((data[p - stride * 3] as u16) << 8).wrapping_add(data[p - stride * 3 + 1] as u16) as i64;
        let pred = predict_value_i64(p1, p2, p3, order) as u16;
        if i & 1 != 0 {
            (pred & 255) as u8
        } else {
            ((pred >> 8) & 255) as u8
        }
    } else {
        let p = start + (i & !3);
        let p1 = decode_uint32(data, pos, p - stride);
        let p2 = decode_uint32(data, pos, p - stride * 2);
        let p3 = decode_uint32(data, pos, p - stride * 3);
        let pred = match order {
            0 => p1,
            1 => p1.wrapping_mul(2).wrapping_sub(p2),
            2 => p1.wrapping_mul(3).wrapping_sub(p2.wrapping_mul(3)).wrapping_add(p3),
            _ => 0,
        };
        let shiftbytes = 3 - (i & 3) as u32;
        ((pred >> (shiftbytes * 8)) & 255) as u8
    }
}

/// Translation of `ICCANSContext()`.
fn icc_ans_context(i: usize, b1: u8, b2: u8) -> usize {
    if i <= 128 {
        return 0;
    }
    1 + byte_kind1(b1) as usize + byte_kind2(b2) as usize * 8
}

// --- icc_codec.cc ---

// Shuffles or interleaves bytes, for example with width 2, turns "ABCDabcd"
// into "AaBbCcDc". Transposes a matrix of ceil(size / width) columns and
// width rows. There are size elements, size may be < width * height, if so the
// last elements of the rightmost column are missing, the missing spots are
// transposed along with the filled spots, and the result has the missing
// elements at the end of the bottom row. The input is the input matrix in
// scanline order but with missing elements skipped (which may occur in multiple
// locations), the output is the result matrix in scanline order (with
// no need to skip missing elements as they are past the end of the data).
/// Translation of `Shuffle()`.
fn shuffle(data: &mut [u8], size: usize, width: usize) {
    let height = size.div_ceil(width); // amount of rows of output
    let mut result = vec![0u8; size];
    // i = output index, j input index
    let mut s = 0usize;
    let mut j = 0usize;
    for r in result.iter_mut() {
        *r = data[j];
        j += height;
        if j >= size {
            s += 1;
            j = s;
        }
    }
    data[..size].copy_from_slice(&result);
}

// TODO(eustas): should be 20, or even 18, once DecodeVarInt is improved;
//               currently DecodeVarInt does not signal the errors, and marks
//               11 bytes as used even if only 10 are used (and 9 is enough for
//               63-bit values).
const K_PREAMBLE_SIZE: usize = 22; // enough for reading 2 VarInts

/// Mimics the beginning of UnpredictICC for quick validity check. At least
/// kPreambleSize bytes of data should be valid at invocation time.
/// Translation of `CheckPreamble()`.
fn check_preamble(data: &PaddedBytes, enc_size: usize, output_limit: usize) -> Status {
    let enc = data.as_slice();
    let size = data.len();
    let mut pos = 0usize;
    let osize = decode_var_int(enc, size, &mut pos);
    check_is_32bit(osize)?;
    if pos >= size {
        return jxl_failure!("Out of bounds");
    }
    let csize = decode_var_int(enc, size, &mut pos);
    check_is_32bit(csize)?;
    check_out_of_bounds(pos, csize, size)?;
    // We expect that UnpredictICC inflates input, not the other way round.
    if osize + 65536 < enc_size as u64 {
        return jxl_failure!("Malformed ICC");
    }
    if output_limit != 0 && osize > output_limit as u64 {
        return jxl_failure!("Decoded ICC is too large");
    }
    Ok(())
}

/// Decodes the result of PredictICC back to a valid ICC profile.
/// Translation of `UnpredictICC()`.
fn unpredict_icc(enc: &[u8], size: usize, result: &mut PaddedBytes) -> Status {
    if !result.is_empty() {
        return jxl_failure!("result must be empty initially");
    }
    let mut pos = 0usize;
    // TODO(lode): technically speaking we need to check that the entire varint
    // decoding never goes out of bounds, not just the first byte. This requires
    // a DecodeVarInt function that returns an error code. It is safe to use
    // DecodeVarInt with out of bounds values, it silently returns, but the
    // specification requires an error. Idem for all DecodeVarInt below.
    if pos >= size {
        return jxl_failure!("Out of bounds");
    }
    let osize = decode_var_int(enc, size, &mut pos); // Output size
    check_is_32bit(osize)?;
    if pos >= size {
        return jxl_failure!("Out of bounds");
    }
    let csize = decode_var_int(enc, size, &mut pos); // Commands size
    // Every command is translated to at least on byte.
    check_is_32bit(csize)?;
    let mut cpos = pos; // pos in commands stream
    check_out_of_bounds(pos, csize, size)?;
    let commands_end = cpos + csize as usize;
    pos = commands_end; // pos in data stream

    // Header
    let mut header = icc_initial_header_prediction();
    encode_uint32(0, osize as u32, &mut header);
    for i in 0..=K_ICC_HEADER_SIZE {
        if result.len() as u64 == osize {
            if cpos != commands_end {
                return jxl_failure!("Not all commands used");
            }
            if pos != size {
                return jxl_failure!("Not all data used");
            }
            return Ok(()); // Valid end
        }
        if i == K_ICC_HEADER_SIZE {
            break; // Done
        }
        let n = result.len();
        icc_predict_header(result, n, &mut header, i);
        if pos >= size {
            return jxl_failure!("Out of bounds");
        }
        result.push(enc[pos].wrapping_add(header[i]));
        pos += 1;
    }
    if cpos >= commands_end {
        return jxl_failure!("Out of bounds");
    }

    // Tag list
    let mut numtags = decode_var_int(enc, size, &mut cpos);

    if numtags != 0 {
        numtags -= 1;
        check_is_32bit(numtags)?;
        append_uint32(numtags as u32, result);
        let mut prevtagstart: u64 = K_ICC_HEADER_SIZE as u64 + numtags * 12;
        let mut prevtagsize: u64 = 0;
        loop {
            if result.len() as u64 > osize {
                return jxl_failure!("Invalid result size");
            }
            if cpos > commands_end {
                return jxl_failure!("Out of bounds");
            }
            if cpos == commands_end {
                break; // Valid end
            }
            let command = enc[cpos];
            cpos += 1;
            let tagcode = command & 63;
            let tag: Tag;
            if tagcode == 0 {
                break;
            } else if tagcode == K_COMMAND_TAG_UNKNOWN {
                check_out_of_bounds(pos, 4, size)?;
                tag = decode_keyword(enc, size, pos);
                pos += 4;
            } else if tagcode == K_COMMAND_TAG_TRC {
                tag = K_RTRC_TAG;
            } else if tagcode == K_COMMAND_TAG_XYZ {
                tag = K_RXYZ_TAG;
            } else {
                if (tagcode - K_COMMAND_TAG_STRING_FIRST) as usize >= K_TAG_STRINGS.len() {
                    return jxl_failure!("Unknown tagcode");
                }
                tag = K_TAG_STRINGS[(tagcode - K_COMMAND_TAG_STRING_FIRST) as usize];
            }
            append_keyword(&tag, result);

            let tagstart: u64;
            let mut tagsize: u64 = prevtagsize;
            if tag == K_RXYZ_TAG
                || tag == K_GXYZ_TAG
                || tag == K_BXYZ_TAG
                || tag == K_KXYZ_TAG
                || tag == K_WTPT_TAG
                || tag == K_BKPT_TAG
                || tag == K_LUMI_TAG
            {
                tagsize = 20;
            }

            if command & K_FLAG_BIT_OFFSET != 0 {
                if cpos >= commands_end {
                    return jxl_failure!("Out of bounds");
                }
                tagstart = decode_var_int(enc, size, &mut cpos);
            } else {
                check_is_32bit(prevtagstart)?;
                tagstart = prevtagstart.wrapping_add(prevtagsize);
            }
            check_is_32bit(tagstart)?;
            append_uint32(tagstart as u32, result);
            if command & K_FLAG_BIT_SIZE != 0 {
                if cpos >= commands_end {
                    return jxl_failure!("Out of bounds");
                }
                tagsize = decode_var_int(enc, size, &mut cpos);
            }
            check_is_32bit(tagsize)?;
            append_uint32(tagsize as u32, result);
            prevtagstart = tagstart;
            prevtagsize = tagsize;

            if tagcode == K_COMMAND_TAG_TRC {
                append_keyword(&K_GTRC_TAG, result);
                append_uint32(tagstart as u32, result);
                append_uint32(tagsize as u32, result);
                append_keyword(&K_BTRC_TAG, result);
                append_uint32(tagstart as u32, result);
                append_uint32(tagsize as u32, result);
            }

            if tagcode == K_COMMAND_TAG_XYZ {
                check_is_32bit(tagstart.wrapping_add(tagsize.wrapping_mul(2)))?;
                append_keyword(&K_GXYZ_TAG, result);
                append_uint32((tagstart + tagsize) as u32, result);
                append_uint32(tagsize as u32, result);
                append_keyword(&K_BXYZ_TAG, result);
                append_uint32((tagstart + tagsize * 2) as u32, result);
                append_uint32(tagsize as u32, result);
            }
        }
    }

    // Main Content
    loop {
        if result.len() as u64 > osize {
            return jxl_failure!("Invalid result size");
        }
        if cpos > commands_end {
            return jxl_failure!("Out of bounds");
        }
        if cpos == commands_end {
            break; // Valid end
        }
        let command = enc[cpos];
        cpos += 1;
        if command == K_COMMAND_INSERT {
            if cpos >= commands_end {
                return jxl_failure!("Out of bounds");
            }
            let num = decode_var_int(enc, size, &mut cpos);
            check_out_of_bounds(pos, num, size)?;
            for _ in 0..num {
                result.push(enc[pos]);
                pos += 1;
            }
        } else if command == K_COMMAND_SHUFFLE2 || command == K_COMMAND_SHUFFLE4 {
            if cpos >= commands_end {
                return jxl_failure!("Out of bounds");
            }
            let num = decode_var_int(enc, size, &mut cpos) as usize;
            check_out_of_bounds(pos, num as u64, size)?;
            let mut shuffled: Vec<u8> = enc[pos..pos + num].to_vec();
            if command == K_COMMAND_SHUFFLE2 {
                shuffle(&mut shuffled, num, 2);
            } else if command == K_COMMAND_SHUFFLE4 {
                shuffle(&mut shuffled, num, 4);
            }
            for &s in &shuffled {
                result.push(s);
                pos += 1;
            }
        } else if command == K_COMMAND_PREDICT {
            check_out_of_bounds(cpos, 2, commands_end)?;
            let flags = enc[cpos];
            cpos += 1;

            let width = (flags & 3) as usize + 1;
            if width == 3 {
                return jxl_failure!("Invalid width");
            }

            let order = ((flags & 12) >> 2) as i32;
            if order == 3 {
                return jxl_failure!("Invalid order");
            }

            let mut stride: u64 = width as u64;
            if flags & 16 != 0 {
                if cpos >= commands_end {
                    return jxl_failure!("Out of bounds");
                }
                stride = decode_var_int(enc, size, &mut cpos);
                if stride < width as u64 {
                    return jxl_failure!("Invalid stride");
                }
            }
            // If stride * 4 >= result->size(), return failure. The check
            // "size == 0 || ((size - 1) >> 2) < stride" corresponds to
            // "stride * 4 >= size", but does not suffer from integer overflow.
            // This check is more strict than necessary but follows the specification
            // and the encoder should ensure this is followed.
            if result.is_empty() || (((result.len() - 1) >> 2) as u64) < stride {
                return jxl_failure!("Invalid stride");
            }

            if cpos >= commands_end {
                return jxl_failure!("Out of bounds");
            }
            let num = decode_var_int(enc, size, &mut cpos) as usize; // in bytes
            check_out_of_bounds(pos, num as u64, size)?;

            let mut shuffled: Vec<u8> = enc[pos..pos + num].to_vec();
            if width > 1 {
                shuffle(&mut shuffled, num, width);
            }

            let start = result.len();
            for (i, &s) in shuffled.iter().enumerate() {
                let predicted = linear_predict_icc_value(result, start, i, stride as usize, width, order);
                result.push(predicted.wrapping_add(s));
            }
            pos += num;
        } else if command == K_COMMAND_XYZ {
            append_keyword(&K_XYZ_TAG, result);
            for _ in 0..4 {
                result.push(0);
            }
            check_out_of_bounds(pos, 12, size)?;
            for _ in 0..12 {
                result.push(enc[pos]);
                pos += 1;
            }
        } else if command >= K_COMMAND_TYPE_START_FIRST
            && command < K_COMMAND_TYPE_START_FIRST + K_TYPE_STRINGS.len() as u8
        {
            append_keyword(&K_TYPE_STRINGS[(command - K_COMMAND_TYPE_START_FIRST) as usize], result);
            for _ in 0..4 {
                result.push(0);
            }
        } else {
            return jxl_failure!("Unknown command");
        }
    }

    if pos != size {
        return jxl_failure!("Not all data used");
    }
    if result.len() as u64 != osize {
        return jxl_failure!("Invalid result size");
    }

    Ok(())
}

/// Translation of `ICCReader::CheckEOI()`.
fn check_eoi(reader: &mut BitReader<'_>) -> Status {
    if reader.all_reads_within_bounds() {
        return Ok(());
    }
    jxl_status!(StatusCode::NotEnoughBytes, "Not enough bytes for reading ICC profile")
}

/// Translation of `ICCReader::Init()` followed by `ICCReader::Process()`
/// (for a first call: `bits_to_skip_` is 0).
pub(crate) fn read_icc(reader: &mut BitReader<'_>, output_limit: usize, icc: &mut PaddedBytes) -> Status {
    // (Init)
    check_eoi(reader)?;
    let used_bits_base = reader.total_bits_consumed();
    let enc_size = u64_coder_read(reader);
    if enc_size > 268435456 {
        // Avoid too large memory allocation for invalid file.
        return jxl_failure!("Too large encoded profile");
    }
    let enc_size = enc_size as usize;
    let mut code = AnsCode::default();
    let mut context_map: Vec<u8> = Vec::new();
    decode_histograms(reader, K_NUM_ICC_CONTEXTS, &mut code, &mut context_map, false)?;
    let mut ans_reader = AnsSymbolReader::new(&code, reader, 0);
    let mut i = 0usize;
    let mut decompressed: PaddedBytes = vec![0; (i + 0x400).min(enc_size)];
    while i < 2usize.min(enc_size) {
        let b1 = if i > 0 { decompressed[i - 1] } else { 0 };
        let b2 = if i > 1 { decompressed[i - 2] } else { 0 };
        decompressed[i] = ans_reader.read_hybrid_uint(icc_ans_context(i, b1, b2), reader, &context_map) as u8;
        i += 1;
    }
    if enc_size > K_PREAMBLE_SIZE {
        while i < K_PREAMBLE_SIZE {
            decompressed[i] = ans_reader.read_hybrid_uint(
                icc_ans_context(i, decompressed[i - 1], decompressed[i - 2]),
                reader,
                &context_map,
            ) as u8;
            i += 1;
        }
        check_eoi(reader)?;
        check_preamble(&decompressed, enc_size, output_limit)?;
    }

    // (Process)
    let mut checkpoint = Checkpoint::default();
    ans_reader.save(&mut checkpoint);
    while i < enc_size {
        if i % AnsSymbolReader::K_MAX_CHECKPOINT_INTERVAL == 0 && i > 0 {
            // (check_and_restore: on failure upstream restores the
            // checkpoint for a later call.)
            check_eoi(reader)?;
            ans_reader.save(&mut checkpoint);
            if i > 0 && (i & 0xFFFF) == 0 {
                let used_bytes = (reader.total_bits_consumed() - used_bits_base) as f32 / 8.0f32;
                if i as f32 > used_bytes * 256.0 {
                    return jxl_failure!("Corrupted stream");
                }
            }
            let new_len = (i + 0x400).min(enc_size);
            decompressed.resize(new_len, 0);
        }
        debug_assert!(i >= 2);
        decompressed[i] = ans_reader.read_hybrid_uint(
            icc_ans_context(i, decompressed[i - 1], decompressed[i - 2]),
            reader,
            &context_map,
        ) as u8;
        i += 1;
    }
    check_eoi(reader)?;
    if !ans_reader.check_ans_final_state() {
        return jxl_failure!("Corrupted ICC profile");
    }

    icc.clear();
    let n = decompressed.len();
    unpredict_icc(&decompressed, n, icc)
}
