// Rust translation of lib/jxl/dec_context_map.h and
// lib/jxl/dec_context_map.cc from libjxl (https://github.com/libjxl/libjxl,
// at the revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Context map decoding.

use super::base::{jxl_failure, Status};
use super::dec_ans::{decode_histograms, AnsCode, AnsSymbolReader};
use super::dec_bit_reader::BitReader;

// Context map uses uint8_t.
pub(crate) const K_MAX_CLUSTERS: usize = 256;

fn move_to_front(v: &mut [u8; 256], index: u8) {
    let value = v[index as usize];
    let mut i = index;
    while i != 0 {
        v[i as usize] = v[i as usize - 1];
        i -= 1;
    }
    v[0] = value;
}

fn inverse_move_to_front_transform(v: &mut [u8]) {
    let mut mtf = [0u8; 256];
    for (i, m) in mtf.iter_mut().enumerate() {
        *m = i as u8;
    }
    for x in v.iter_mut() {
        let index = *x;
        *x = mtf[index as usize];
        if index != 0 {
            move_to_front(&mut mtf, index);
        }
    }
}

fn verify_context_map(context_map: &[u8], num_htrees: usize) -> Status {
    let mut have_htree = vec![false; num_htrees];
    let mut num_found = 0;
    for &htree in context_map {
        if htree as usize >= num_htrees {
            return jxl_failure!("Invalid histogram index in context map.");
        }
        if !have_htree[htree as usize] {
            have_htree[htree as usize] = true;
            num_found += 1;
        }
    }
    if num_found != num_htrees {
        return jxl_failure!("Incomplete context map.");
    }
    Ok(())
}

/// Reads the context map from the bit stream. On calling this function,
/// context_map->size() must be the number of possible context ids.
/// Sets *num_htrees to the number of different histogram ids in
/// *context_map. Translation of `DecodeContextMap()`.
pub(crate) fn decode_context_map(
    context_map: &mut [u8],
    num_htrees: &mut usize,
    input: &mut BitReader<'_>,
) -> Status {
    let is_simple = input.read_fixed_bits::<1>() != 0;
    if is_simple {
        let bits_per_entry = input.read_fixed_bits::<2>() as usize;
        if bits_per_entry != 0 {
            for c in context_map.iter_mut() {
                *c = input.read_bits(bits_per_entry) as u8;
            }
        } else {
            context_map.fill(0);
        }
    } else {
        let use_mtf = input.read_fixed_bits::<1>() != 0;
        let mut code = AnsCode::default();
        let mut dummy_ctx_map: Vec<u8> = Vec::new();
        // Usage of LZ77 is disallowed if decoding only two symbols. This doesn't
        // make sense in non-malicious bitstreams, and could cause a stack overflow
        // in malicious bitstreams by making every context map require its own
        // context map.
        decode_histograms(
            input,
            1,
            &mut code,
            &mut dummy_ctx_map,
            /*disallow_lz77=*/ context_map.len() <= 2,
        )?;
        let mut reader = AnsSymbolReader::new(&code, input, 0);
        let mut i = 0;
        while i < context_map.len() {
            let sym = reader.read_hybrid_uint(0, input, &dummy_ctx_map) as u32;
            if sym as usize >= K_MAX_CLUSTERS {
                return jxl_failure!("Invalid cluster ID");
            }
            context_map[i] = sym as u8;
            i += 1;
        }
        if !reader.check_ans_final_state() {
            return jxl_failure!("Invalid context map");
        }
        if use_mtf {
            inverse_move_to_front_transform(context_map);
        }
    }
    *num_htrees = context_map.iter().copied().max().unwrap_or(0) as usize + 1;
    verify_context_map(context_map, *num_htrees)
}
