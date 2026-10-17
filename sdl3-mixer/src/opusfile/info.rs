// Rust translation of src/info.c (the ID and comment header parsers and the
// R128 gain tags) from opusfile 0.12 (https://opus-codec.org, at the revision
// SDL_mixer's external/opusfile pins).
// THE libopusfile SOURCE CODE IS (C) COPYRIGHT 2012-2020
// by the Xiph.Org Foundation and contributors https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Opus header parsing. The memory allocations a header's sizes ask for
//! are fallible, failing with `OP_EFAULT` as C's `malloc()` failures do.

use super::internal::op_strncasecmp;
use super::{
    OpusHead, OpusTags, OP_EBADHEADER, OP_EFAULT, OP_EIMPL, OP_ENOTFORMAT, OP_EVERSION, OP_FALSE,
};

const INT_MAX: u32 = i32::MAX as u32;

fn op_parse_uint16le(data: &[u8]) -> u32 {
    data[0] as u32 | (data[1] as u32) << 8
}

fn op_parse_int16le(data: &[u8]) -> i32 {
    let ret = data[0] as i32 | (data[1] as i32) << 8;
    (ret ^ 0x8000) - 0x8000
}

fn op_parse_uint32le(data: &[u8]) -> u32 {
    data[0] as u32 | (data[1] as u32) << 8 | (data[2] as u32) << 16 | (data[3] as u32) << 24
}

/// Translation of `opus_head_parse()`.
pub(crate) fn opus_head_parse(head_out: Option<&mut OpusHead>, data: &[u8]) -> i32 {
    let len = data.len();
    let mut head = OpusHead::default();
    if len < 8 {
        return OP_ENOTFORMAT;
    }
    if &data[..8] != b"OpusHead" {
        return OP_ENOTFORMAT;
    }
    if len < 9 {
        return OP_EBADHEADER;
    }
    head.version = data[8] as i32;
    if head.version > 15 {
        return OP_EVERSION;
    }
    if len < 19 {
        return OP_EBADHEADER;
    }
    head.channel_count = data[9] as i32;
    head.pre_skip = op_parse_uint16le(&data[10..]);
    head.input_sample_rate = op_parse_uint32le(&data[12..]);
    head.output_gain = op_parse_int16le(&data[16..]);
    head.mapping_family = data[18] as i32;
    let mut mapping = [0u8; super::OPUS_CHANNEL_COUNT_MAX];
    let mapping_len;
    if head.mapping_family == 0 {
        if head.channel_count < 1 || head.channel_count > 2 {
            return OP_EBADHEADER;
        }
        if head.version <= 1 && len > 19 {
            return OP_EBADHEADER;
        }
        head.stream_count = 1;
        head.coupled_count = head.channel_count - 1;
        mapping[0] = 0;
        mapping[1] = 1;
        mapping_len = 2;
    } else if head.mapping_family == 1 {
        if head.channel_count < 1 || head.channel_count > 8 {
            return OP_EBADHEADER;
        }
        let size = 21 + head.channel_count as usize;
        if len < size || head.version <= 1 && len > size {
            return OP_EBADHEADER;
        }
        head.stream_count = data[19] as i32;
        if head.stream_count < 1 {
            return OP_EBADHEADER;
        }
        head.coupled_count = data[20] as i32;
        if head.coupled_count > head.stream_count {
            return OP_EBADHEADER;
        }
        for ci in 0..head.channel_count as usize {
            if data[21 + ci] as i32 >= head.stream_count + head.coupled_count
                && data[21 + ci] != 255
            {
                return OP_EBADHEADER;
            }
        }
        mapping_len = head.channel_count as usize;
        mapping[..mapping_len].copy_from_slice(&data[21..21 + mapping_len]);
    }
    /*General purpose players should not attempt to play back content with
    channel mapping family 255.*/
    else if head.mapping_family == 255 {
        return OP_EIMPL;
    }
    /*No other channel mapping families are currently defined.*/
    else {
        return OP_EBADHEADER;
    }
    if let Some(out) = head_out {
        /* (C copies the fields but leaves the rest of the mapping alone) */
        let old_mapping = out.mapping;
        *out = head;
        out.mapping = old_mapping;
        out.mapping[..mapping_len].copy_from_slice(&mapping[..mapping_len]);
    }
    0
}

/// Translation of `opus_tags_clear()` (and `opus_tags_init()`).
pub(crate) fn opus_tags_clear(tags: &mut OpusTags) {
    *tags = OpusTags::default();
}

/*Duplicate a (possibly non-NUL terminated) string with a known length.*/
/// Translation of `op_strdup_with_len()`; `None` is an allocation failure.
fn op_strdup_with_len(s: &[u8]) -> Option<Vec<u8>> {
    let mut ret = Vec::new();
    ret.try_reserve_exact(s.len() + 1).ok()?;
    ret.extend_from_slice(s);
    Some(ret)
}

/*The actual implementation of opus_tags_parse().
Unlike the public API, this function requires _tags to already be
initialized, modifies its contents before success is guaranteed, and assumes
the caller will clear it on error.*/
fn opus_tags_parse_impl(mut tags: Option<&mut OpusTags>, mut data: &[u8]) -> i32 {
    let mut len = data.len();
    if len < 8 {
        return OP_ENOTFORMAT;
    }
    if &data[..8] != b"OpusTags" {
        return OP_ENOTFORMAT;
    }
    if len < 16 {
        return OP_EBADHEADER;
    }
    data = &data[8..];
    len -= 8;
    let mut count = op_parse_uint32le(data);
    data = &data[4..];
    len -= 4;
    if count as usize > len {
        return OP_EBADHEADER;
    }
    if let Some(tags) = tags.as_deref_mut() {
        match op_strdup_with_len(&data[..count as usize]) {
            Some(v) => tags.vendor = v,
            None => return OP_EFAULT,
        }
    }
    data = &data[count as usize..];
    len -= count as usize;
    if len < 4 {
        return OP_EBADHEADER;
    }
    count = op_parse_uint32le(data);
    data = &data[4..];
    len -= 4;
    /*Check to make sure there's minimally sufficient data left in the packet.*/
    if count as usize > len >> 2 {
        return OP_EBADHEADER;
    }
    /*Check for overflow (the API limits this to an int).*/
    if count > INT_MAX - 1 {
        return OP_EFAULT;
    }
    if let Some(tags) = tags.as_deref_mut() {
        /* (op_tags_ensure_capacity()) */
        if count >= INT_MAX
            || tags
                .user_comments
                .try_reserve_exact(count as usize)
                .is_err()
        {
            return OP_EFAULT;
        }
    }
    let ncomments = count as usize;
    for ci in 0..ncomments {
        /*Check to make sure there's minimally sufficient data left in the packet.*/
        if ncomments - ci > len >> 2 {
            return OP_EBADHEADER;
        }
        count = op_parse_uint32le(data);
        data = &data[4..];
        len -= 4;
        if count as usize > len {
            return OP_EBADHEADER;
        }
        /*Check for overflow (the API limits this to an int).*/
        if count > INT_MAX {
            return OP_EFAULT;
        }
        if let Some(tags) = tags.as_deref_mut() {
            match op_strdup_with_len(&data[..count as usize]) {
                Some(v) => tags.user_comments.push(v),
                None => return OP_EFAULT,
            }
            tags.comments = ci as i32 + 1;
        }
        data = &data[count as usize..];
        len -= count as usize;
    }
    if len > 0 && (data[0] & 1) != 0 {
        if len > INT_MAX as usize {
            return OP_EFAULT;
        }
        if let Some(tags) = tags.as_deref_mut() {
            match op_strdup_with_len(&data[..len]) {
                Some(v) => tags.binary_suffix = v,
                None => return OP_EFAULT,
            }
        }
    }
    0
}

/// Translation of `opus_tags_parse()`.
pub(crate) fn opus_tags_parse(tags_out: Option<&mut OpusTags>, data: &[u8]) -> i32 {
    if let Some(tags_out) = tags_out {
        let mut tags = OpusTags::default();
        let ret = opus_tags_parse_impl(Some(&mut tags), data);
        if ret >= 0 {
            *tags_out = tags;
        }
        ret
    } else {
        opus_tags_parse_impl(None, data)
    }
}

/// Translation of `opus_tagncompare()`.
pub(crate) fn opus_tagncompare(tag_name: &[u8], tag_len: i32, comment: &[u8]) -> i32 {
    debug_assert!(tag_len >= 0);
    let ret = op_strncasecmp(tag_name, comment, tag_len);
    if ret != 0 {
        ret
    } else {
        b'=' as i32 - comment.get(tag_len as usize).copied().unwrap_or(0) as i8 as i32
    }
}

fn opus_tags_get_gain(tags: &OpusTags, gain_q8: &mut i32, tag_name: &[u8], tag_len: usize) -> i32 {
    let comments = &tags.user_comments;
    let ncomments = tags.comments as usize;
    /*Look for the first valid tag with the name _tag_name and use that.*/
    for comment in comments[..ncomments].iter() {
        debug_assert!(tag_len <= i32::MAX as usize);
        if opus_tagncompare(tag_name, tag_len as i32, comment) == 0 {
            /* (reading past the comment reads C's NUL terminator) */
            let at = |i: usize| comment.get(i).copied().unwrap_or(0);
            let mut p = tag_len + 1;
            let mut negative = 0;
            if at(p) == b'-' {
                negative = -1;
                p += 1;
            } else if at(p) == b'+' {
                p += 1;
            }
            let mut g: i32 = 0;
            while at(p).is_ascii_digit() {
                g = 10 * g + (at(p) - b'0') as i32;
                if g > 32767 - negative {
                    break;
                }
                p += 1;
            }
            /*This didn't look like a signed 16-bit decimal integer.
            Not a valid gain tag.*/
            if at(p) != 0 {
                continue;
            }
            *gain_q8 = (g + negative) ^ negative;
            return 0;
        }
    }
    OP_FALSE
}

/// Translation of `opus_tags_get_album_gain()`.
pub(crate) fn opus_tags_get_album_gain(tags: &OpusTags, gain_q8: &mut i32) -> i32 {
    opus_tags_get_gain(tags, gain_q8, b"R128_ALBUM_GAIN", 15)
}

/// Translation of `opus_tags_get_track_gain()`.
pub(crate) fn opus_tags_get_track_gain(tags: &OpusTags, gain_q8: &mut i32) -> i32 {
    opus_tags_get_gain(tags, gain_q8, b"R128_TRACK_GAIN", 15)
}
