// Rust translation of the decoder's part of src/libFLAC/format.c from
// libFLAC 1.3.4 (https://xiph.org/flac/, at the revision SDL_mixer's
// external/flac pins).
// Copyright (C) 2000-2009  Josh Coalson
// Copyright (C) 2011-2016  Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The format's constants and the partitioned Rice contents' memory
//! management. The validity checks (`FLAC__format_sample_rate_is_valid()`
//! and the like), the seek table and cue sheet utilities, the strings and
//! the version and vendor strings are the encoder's and the metadata
//! interfaces', and aren't translated.

use super::PartitionedRiceContents;

pub(crate) const FLAC__STREAM_SYNC_STRING: [u8; 4] = *b"fLaC";

pub(crate) const FLAC__STREAM_METADATA_STREAMINFO_MIN_BLOCK_SIZE_LEN: u32 = 16; /* bits */
pub(crate) const FLAC__STREAM_METADATA_STREAMINFO_MAX_BLOCK_SIZE_LEN: u32 = 16; /* bits */
pub(crate) const FLAC__STREAM_METADATA_STREAMINFO_MIN_FRAME_SIZE_LEN: u32 = 24; /* bits */
pub(crate) const FLAC__STREAM_METADATA_STREAMINFO_MAX_FRAME_SIZE_LEN: u32 = 24; /* bits */
pub(crate) const FLAC__STREAM_METADATA_STREAMINFO_SAMPLE_RATE_LEN: u32 = 20; /* bits */
pub(crate) const FLAC__STREAM_METADATA_STREAMINFO_CHANNELS_LEN: u32 = 3; /* bits */
pub(crate) const FLAC__STREAM_METADATA_STREAMINFO_BITS_PER_SAMPLE_LEN: u32 = 5; /* bits */
pub(crate) const FLAC__STREAM_METADATA_STREAMINFO_TOTAL_SAMPLES_LEN: u32 = 36; /* bits */

pub(crate) const FLAC__STREAM_METADATA_APPLICATION_ID_LEN: u32 = 32; /* bits */

pub(crate) const FLAC__STREAM_METADATA_SEEKPOINT_SAMPLE_NUMBER_LEN: u32 = 64; /* bits */
pub(crate) const FLAC__STREAM_METADATA_SEEKPOINT_STREAM_OFFSET_LEN: u32 = 64; /* bits */
pub(crate) const FLAC__STREAM_METADATA_SEEKPOINT_FRAME_SAMPLES_LEN: u32 = 16; /* bits */

pub(crate) const FLAC__STREAM_METADATA_SEEKPOINT_PLACEHOLDER: u64 = 0xffff_ffff_ffff_ffff;

pub(crate) const FLAC__STREAM_METADATA_IS_LAST_LEN: u32 = 1; /* bits */
pub(crate) const FLAC__STREAM_METADATA_TYPE_LEN: u32 = 7; /* bits */
pub(crate) const FLAC__STREAM_METADATA_LENGTH_LEN: u32 = 24; /* bits */

pub(crate) const FLAC__FRAME_FOOTER_CRC_LEN: u32 = 16; /* bits */

pub(crate) const FLAC__ENTROPY_CODING_METHOD_TYPE_LEN: u32 = 2; /* bits */
pub(crate) const FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_ORDER_LEN: u32 = 4; /* bits */
pub(crate) const FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_PARAMETER_LEN: u32 = 4; /* bits */
pub(crate) const FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2_PARAMETER_LEN: u32 = 5; /* bits */
pub(crate) const FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_RAW_LEN: u32 = 5; /* bits */

pub(crate) const FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_ESCAPE_PARAMETER: u32 = 15; /* == (1<<FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_PARAMETER_LEN)-1 */
pub(crate) const FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2_ESCAPE_PARAMETER: u32 = 31; /* == (1<<FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2_PARAMETER_LEN)-1 */

pub(crate) const FLAC__SUBFRAME_LPC_QLP_COEFF_PRECISION_LEN: u32 = 4; /* bits */
pub(crate) const FLAC__SUBFRAME_LPC_QLP_SHIFT_LEN: u32 = 5; /* bits */

/// `FLAC__format_entropy_coding_method_partitioned_rice_contents_init()`.
pub(crate) fn FLAC__format_entropy_coding_method_partitioned_rice_contents_init(
    object: &mut PartitionedRiceContents,
) {
    object.parameters = Vec::new();
    object.raw_bits = Vec::new();
    object.capacity_by_order = 0;
}

/// `FLAC__format_entropy_coding_method_partitioned_rice_contents_clear()`.
pub(crate) fn FLAC__format_entropy_coding_method_partitioned_rice_contents_clear(
    object: &mut PartitionedRiceContents,
) {
    FLAC__format_entropy_coding_method_partitioned_rice_contents_init(object);
}

/// `FLAC__format_entropy_coding_method_partitioned_rice_contents_ensure_size()`
/// (`false` when the arrays can't be allocated).
pub(crate) fn FLAC__format_entropy_coding_method_partitioned_rice_contents_ensure_size(
    object: &mut PartitionedRiceContents,
    max_partition_order: u32,
) -> bool {
    if object.capacity_by_order < max_partition_order {
        let n = 1usize << max_partition_order;
        let Some(parameters) = grow(std::mem::take(&mut object.parameters), n) else {
            return false;
        };
        object.parameters = parameters;
        let Some(mut raw_bits) = grow(std::mem::take(&mut object.raw_bits), n) else {
            return false;
        };
        raw_bits.fill(0);
        object.raw_bits = raw_bits;
        object.capacity_by_order = max_partition_order;
    }

    true
}

/// `safe_realloc_()` of a `uint32_t` array to `n` elements (`None`, the old
/// array freed, when that fails).
fn grow(mut v: Vec<u32>, n: usize) -> Option<Vec<u32>> {
    v.try_reserve_exact(n.saturating_sub(v.len())).ok()?;
    v.resize(n, 0);
    Some(v)
}
