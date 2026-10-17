// Rust translation of aom_dsp/bitwriter.c and aom_dsp/bitwriter.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The symbol writer over the range encoder (CONFIG_RD_DEBUG and
//! CONFIG_BITSTREAM_DEBUG are off).

use super::entenc::{
    od_ec_enc_clear, od_ec_enc_done, od_ec_enc_init, od_ec_enc_tell, od_ec_encode_bool_q15,
    od_ec_encode_cdf_q15, OdEcEnc,
};
use super::prob::{update_cdf, AomCdfProb};

/// Translation of `struct aom_writer`; `buffer` is the caller's, given to
/// `aom_stop_encode()`.
#[derive(Debug, Default)]
pub(crate) struct AomWriter {
    pub(crate) pos: u32,
    pub(crate) ec: OdEcEnc,
    pub(crate) allow_update_cdf: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TokenStats {
    pub(crate) cost: i32,
}

pub(crate) fn init_token_stats(token_stats: &mut TokenStats) {
    token_stats.cost = 0;
}

pub(crate) fn aom_start_encode(w: &mut AomWriter) {
    w.pos = 0;
    od_ec_enc_init(&mut w.ec, 62025);
}

/// Translation of `aom_stop_encode()`: the data goes to the start of
/// `buffer`.
pub(crate) fn aom_stop_encode(w: &mut AomWriter, buffer: &mut [u8]) -> i32 {
    let nb_bits: i32;
    let mut bytes: u32 = 0;
    let data = od_ec_enc_done(&mut w.ec, &mut bytes);
    nb_bits = od_ec_enc_tell(&w.ec);
    if let Some(data) = data {
        buffer[..bytes as usize].copy_from_slice(&w.ec.buf[data..data + bytes as usize]);
    }
    w.pos = bytes;
    od_ec_enc_clear(&mut w.ec);
    nb_bits
}

pub(crate) fn aom_tell_size(w: &AomWriter) -> i32 {
    let nb_bits = od_ec_enc_tell(&w.ec);
    nb_bits
}

#[inline]
pub(crate) fn aom_write(w: &mut AomWriter, bit: i32, probability: i32) {
    let p = (0x7FFFFF - (probability << 15) + probability) >> 8;
    od_ec_encode_bool_q15(&mut w.ec, bit, p as u32);
}

#[inline]
pub(crate) fn aom_write_bit(w: &mut AomWriter, bit: i32) {
    aom_write(w, bit, 128); // aom_prob_half
}

pub(crate) fn aom_write_literal(w: &mut AomWriter, data: i32, bits: i32) {
    let mut bit = bits - 1;
    while bit >= 0 {
        aom_write_bit(w, 1 & (data >> bit));
        bit -= 1;
    }
}

#[inline]
pub(crate) fn aom_write_cdf(w: &mut AomWriter, symb: i32, cdf: &[AomCdfProb], nsymbs: i32) {
    od_ec_encode_cdf_q15(&mut w.ec, symb, cdf, nsymbs);
}

#[inline]
pub(crate) fn aom_write_symbol(w: &mut AomWriter, symb: i32, cdf: &mut [AomCdfProb], nsymbs: i32) {
    aom_write_cdf(w, symb, cdf, nsymbs);
    if w.allow_update_cdf != 0 {
        update_cdf(cdf, symb as i8, nsymbs);
    }
}
