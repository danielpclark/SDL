//! A Rust translation of the SILK decoder of libopus 1.4 (`silk/`), the
//! fixed-point SILK sources that libopus's float build also uses (the
//! `silk/float/` directory only holds the encoder). The plain C paths are
//! translated; none of the `silk/x86`, `silk/arm` or `silk/mips` variants.

pub(crate) mod cng;
pub(crate) mod dec_api;
pub(crate) mod decode_core;
pub(crate) mod decode_frame;
pub(crate) mod decode_indices;
pub(crate) mod decode_parameters;
pub(crate) mod decode_pulses;
pub(crate) mod define;
pub(crate) mod lpc_analysis_filter;
pub(crate) mod nlsf;
pub(crate) mod plc;
pub(crate) mod resampler;
pub(crate) mod sigproc_fix;
pub(crate) mod stereo;
pub(crate) mod structs;
pub(crate) mod sum_sqr_shift;
pub(crate) mod tables;
