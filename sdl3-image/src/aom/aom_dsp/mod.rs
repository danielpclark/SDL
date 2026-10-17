// (the aom_dsp functions the encoder reaches, their plain-C versions: the
// generic target's run-time CPU dispatch picks the `_c` functions)
pub(crate) mod aom_dsp_common;
pub(crate) mod avg;
pub(crate) mod bitwriter;
pub(crate) mod bitwriter_buffer;
pub(crate) mod entenc;
pub(crate) mod fwd_txfm;
pub(crate) mod intrapred;
pub(crate) mod prob;
pub(crate) mod quantize;
pub(crate) mod subtract;
pub(crate) mod variance;
