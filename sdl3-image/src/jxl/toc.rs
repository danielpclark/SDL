// Rust translation of lib/jxl/toc.h and lib/jxl/toc.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The table of contents of a frame.

use super::ac_strategy::CoeffOrderT;
use super::base::{jxl_failure, jxl_status, Status, StatusCode, K_BITS_PER_BYTE};
use super::coeff_order::decode_permutation;
use super::dec_bit_reader::BitReader;
use super::fields::{bits, bits_offset, u32_coder_read, U32Enc};

// (2+bits) = 2,3,4 bytes so encoders can patch TOC after encoding.
// 30 is sufficient for 4K channels of uncompressed 16-bit samples.
pub(crate) const K_TOC_DIST: U32Enc = U32Enc::new(
    bits(10),
    bits_offset(14, 1024),
    bits_offset(22, 17408),
    bits_offset(30, 4211712),
);

// TODO(veluca): move these to FrameDimensions.
/// Translation of `AcGroupIndex()`.
#[inline]
pub(crate) fn ac_group_index(pass: usize, group: usize, num_groups: usize, num_dc_groups: usize, has_ac_global: bool) -> usize {
    1 + num_dc_groups + has_ac_global as usize + pass * num_groups + group
}

/// Translation of `NumTocEntries()`.
#[inline]
pub(crate) fn num_toc_entries(num_groups: usize, num_dc_groups: usize, num_passes: usize, has_ac_global: bool) -> usize {
    if num_groups == 1 && num_passes == 1 {
        return 1;
    }
    ac_group_index(0, 0, num_groups, num_dc_groups, has_ac_global)
        .wrapping_add(num_groups.wrapping_mul(num_passes))
}

/// Translation of `ReadToc()`.
pub(crate) fn read_toc(
    toc_entries: usize,
    reader: &mut BitReader<'_>,
    sizes: &mut Vec<u32>,
    permutation: &mut Vec<CoeffOrderT>,
) -> Status {
    if toc_entries > 65536 {
        // Prevent out of memory if invalid JXL codestream causes a bogus amount
        // of toc_entries such as 2720436919446 to be computed.
        // TODO(lode): verify whether 65536 is a reasonable upper bound
        return jxl_failure!("too many toc entries");
    }

    sizes.clear();
    sizes.resize(toc_entries, 0);
    if reader.total_bits_consumed() >= reader.total_bytes() * K_BITS_PER_BYTE {
        return jxl_status!(StatusCode::NotEnoughBytes, "Not enough bytes for TOC");
    }
    let check_bit_budget = |reader: &BitReader<'_>, num_entries: usize| -> Status {
        // U32Coder reads 2 bits to recognize variant and kTocDist cheapest variant
        // is Bits(10), this way at least 12 bits are required per toc-entry.
        let minimal_bit_cost = num_entries * (2 + 10);
        let bit_budget = reader.total_bytes() * 8;
        let expenses = reader.total_bits_consumed();
        if (expenses <= bit_budget) && (minimal_bit_cost <= bit_budget - expenses) {
            return Ok(());
        }
        jxl_status!(StatusCode::NotEnoughBytes, "Not enough bytes for TOC")
    };

    if reader.read_fixed_bits::<1>() == 1 {
        check_bit_budget(reader, toc_entries)?;
        permutation.resize(toc_entries, 0);
        decode_permutation(/*skip=*/ 0, toc_entries, Some(permutation), reader)?;
    }
    reader.jump_to_byte_boundary()?;
    check_bit_budget(reader, toc_entries)?;
    for s in sizes.iter_mut() {
        *s = u32_coder_read(K_TOC_DIST, reader);
    }
    reader.jump_to_byte_boundary()?;
    check_bit_budget(reader, 0)?;
    Ok(())
}

/// Translation of `ReadGroupOffsets()`.
#[allow(dead_code)]
pub(crate) fn read_group_offsets(
    toc_entries: usize,
    reader: &mut BitReader<'_>,
    offsets: &mut Vec<u64>,
    sizes: &mut Vec<u32>,
    total_size: Option<&mut u64>,
) -> Status {
    let mut permutation: Vec<CoeffOrderT> = Vec::new();
    read_toc(toc_entries, reader, sizes, &mut permutation)?;

    offsets.clear();
    offsets.resize(toc_entries, 0);

    // Prefix sum starting with 0 and ending with the offset of the last group
    let mut offset: u64 = 0;
    for i in 0..toc_entries {
        if offset.wrapping_add(sizes[i] as u64) < offset {
            return jxl_failure!("group offset overflow");
        }
        offsets[i] = offset;
        offset += sizes[i] as u64;
    }
    if let Some(t) = total_size {
        *t = offset;
    }

    if !permutation.is_empty() {
        let mut permuted_offsets: Vec<u64> = Vec::with_capacity(toc_entries);
        let mut permuted_sizes: Vec<u32> = Vec::with_capacity(toc_entries);
        for &index in &permutation {
            permuted_offsets.push(offsets[index as usize]);
            permuted_sizes.push(sizes[index as usize]);
        }
        std::mem::swap(offsets, &mut permuted_offsets);
        std::mem::swap(sizes, &mut permuted_sizes);
    }

    Ok(())
}
