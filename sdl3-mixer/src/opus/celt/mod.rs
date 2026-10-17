// The CELT layer of libopus 1.4 (https://opus-codec.org, at the revision
// SDL_mixer's external/opus pins), translated to Rust.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CELT, the MDCT-based layer of Opus: the decoder, in the floating-point
//! build. Each module translates the libopus file(s) it names.

pub(crate) mod arch;
pub(crate) mod bands;
#[allow(clippy::module_inception)]
pub(crate) mod celt;
pub(crate) mod celt_decoder;
pub(crate) mod celt_lpc;
pub(crate) mod cwrs;
pub(crate) mod entcode;
pub(crate) mod kiss_fft;
pub(crate) mod laplace;
pub(crate) mod mathops;
pub(crate) mod mdct;
pub(crate) mod modes;
pub(crate) mod pitch;
pub(crate) mod quant_bands;
pub(crate) mod rate;
pub(crate) mod static_modes_float;
pub(crate) mod vq;
