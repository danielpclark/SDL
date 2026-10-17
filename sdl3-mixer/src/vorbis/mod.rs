// Rust translation of the declarations of include/vorbis/codec.h,
// include/vorbis/vorbisfile.h, lib/codec_internal.h, lib/backends.h,
// lib/codebook.h, lib/registry.h, lib/misc.h and lib/scales.h from libvorbis
// 1.3.7 (https://xiph.org/vorbis/, at the revision SDL_mixer's
// external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A Rust translation of the decoding half of libvorbis 1.3.7 and of
//! libvorbisfile, as SDL_mixer's CMake builds the copies it vendors
//! (`SDLMIXER_VORBIS_VORBISFILE`, on by default): header and comment
//! parsing (info.c), codebooks (codebook.c, sharedbook.c), floor 0 with
//! its LSP curve (floor0.c and the plain float `vorbis_lsp_to_curve()` of
//! lsp.c, as the build has neither `FLOAT_LOOKUP` nor `INT_LOOKUP`), floor
//! 1 (floor1.c), residues 0, 1 and 2 (res0.c), mapping 0 (mapping0.c), the
//! inverse MDCT (mdct.c), the windows (window.c), block lapping and the PCM
//! buffer (block.c), packet decoding (synthesis.c), and the vorbisfile.c
//! functions SDL_mixer's VORBIS decoder calls (opening, chained streams,
//! `ov_read_float()`, `ov_raw_seek()`, `ov_pcm_seek()`, `ov_pcm_total()`,
//! `ov_info()`, `ov_comment()`).
//!
//! Not translated, as only the encoder uses them: analysis.c, bitrate.c,
//! envelope.c, lpc.c, psy.c, smallft.c, vorbisenc.c, the encoding halves
//! of the files above (the `*_pack()`, `*_forward()`, `*_class()`,
//! `floor1_fit()`/`floor1_encode()` and `vorbis_book_init_encode()`
//! functions, `vorbis_lpc_to_lsp()`, `_vorbis_apply_window()`), the tables
//! only they use (masking.h, the books and modes of lib/books and
//! lib/modes), and lookup.c, which only the `FLOAT_LOOKUP` and
//! `INT_LOOKUP` versions of lsp.c use. Of vorbisfile.c, the stdio opening
//! (`ov_open()`, `ov_fopen()`), `ov_read()`/`ov_read_filter()` (SDL_mixer
//! reads floats with `ov_read_float()`, so the float-to-int packing isn't
//! reached), the time and bitrate queries, page-granularity and lapped
//! seeks, crosslapping and half-rate decoding aren't translated either:
//! SDL_mixer calls none of them.
//!
//! The math is the one of a GCC build for x86-64 Linux with SSE2 floats:
//! `float` expressions are evaluated in `float` and the promotions to
//! `double` happen where C's rules put them (each is spelled out);
//! `cos()`, `sin()`, `atan()`, `exp()`, `log()`, `pow()` and `sqrt()` are
//! SDL's (sdl3's `stdlib::math`, translations of its fdlibm) rather than
//! the C library's; conversions of out-of-range floating point values to
//! integers give x86's "integer indefinite" (`INT_MIN`/`LONG_MIN`), and
//! C's `long` is 64 bits wide.

// The translation keeps upstream's loops, declarations and arithmetic,
// which these lints would have rewritten.
#![allow(
    non_snake_case,
    clippy::assign_op_pattern,
    clippy::collapsible_else_if,
    clippy::field_reassign_with_default,
    clippy::int_plus_one,
    clippy::manual_range_contains,
    clippy::misrefactored_assign_op,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::while_immutable_condition
)]

pub(crate) mod block;
pub(crate) mod codebook;
pub(crate) mod floor0;
pub(crate) mod floor1;
pub(crate) mod info;
pub(crate) mod lsp;
pub(crate) mod mapping0;
pub(crate) mod mdct;
pub(crate) mod res0;
pub(crate) mod sharedbook;
pub(crate) mod synthesis;
pub(crate) mod vorbisfile;
pub(crate) mod window;

use mdct::MdctLookup;

/* codec.h *************************************************************/

/// Translation of `vorbis_info`. `codec_setup` is `None` where C's is
/// NULL.
#[derive(Debug, Default)]
pub(crate) struct VorbisInfo {
    pub version: i32,
    pub channels: i32,
    pub rate: i64,

    /* The below bitrate declarations are *hints*.
    Combinations of the three values carry the following implications:

    all three set to the same value:
      implies a fixed rate bitstream
    only nominal set:
      implies a VBR stream that averages the nominal bitrate.  No hard
      upper/lower limit
    upper and or lower set:
      implies a VBR bitstream that obeys the bitrate limits. nominal
      may also be set to give a nominal rate.
    none set:
      the coder does not care to speculate.
    */
    pub bitrate_upper: i64,
    pub bitrate_nominal: i64,
    pub bitrate_lower: i64,
    /* (bitrate_window is never set) */
    pub codec_setup: Option<Box<CodecSetupInfo>>,
}

/* vorbis_dsp_state buffers the current vorbis audio
analysis/synthesis state.  The DSP state belongs to a specific
logical bitstream ****************************************************/
/// Translation of `vorbis_dsp_state` (synthesis). C's `vi` pointer is the
/// index of the [`VorbisInfo`] in the caller's list (the vorbisfile's
/// links), `None` where C's is NULL; the functions take that info as a
/// parameter. `pcmret` is `pcm` from `pcm_returned` on, so it isn't kept.
#[derive(Debug, Default)]
pub(crate) struct VorbisDspState {
    /* (analysisp is always 0: only synthesis is translated) */
    pub vi: Option<usize>,

    pub pcm: Vec<Vec<f32>>,
    pub pcm_storage: i32,
    pub pcm_current: i32,
    pub pcm_returned: i32,

    /* (preextrapolate is the encoder's) */
    pub eofflag: i32,

    pub lW: i64,
    pub W: i64,
    pub nW: i64,
    pub centerW: i64,

    pub granulepos: i64,
    pub sequence: i64,

    pub glue_bits: i64,
    pub time_bits: i64,
    pub floor_bits: i64,
    pub res_bits: i64,

    pub backend_state: Option<Box<PrivateState>>,
}

/// Translation of `vorbis_block` (synthesis). Its `opb` is made by
/// `vorbis_synthesis()` over the packet for the length of the call, its
/// `vd` is passed where it's used, and its local storage (`localstore`
/// and friends, `_vorbis_block_alloc()`) is ordinary vectors; `pcm` is
/// kept from block to block, as C's local storage is, and `pcm_null` is
/// C's `pcm == NULL` (after `vorbis_synthesis_trackonly()`).
#[derive(Debug, Default)]
pub(crate) struct VorbisBlock {
    /* necessary stream state for linking to the framing abstraction */
    pub pcm: Vec<Vec<f32>>, /* this is a pointer into local storage */
    pub pcm_null: bool,

    pub lW: i64,
    pub W: i64,
    pub nW: i64,
    pub pcmend: i32,
    pub mode: i32,

    pub eofflag: i32,
    pub granulepos: i64,
    pub sequence: i64,

    /* bitmetrics for the frame */
    pub glue_bits: i64,
    pub time_bits: i64,
    pub floor_bits: i64,
    pub res_bits: i64,
}

/* the comments are not part of vorbis_info so that vorbis_info can be
static storage */
/// Translation of `vorbis_comment`. C's `user_comments`/`comment_lengths`
/// pairs are `user_comments` (the comments as stored, without C's added
/// NUL); an empty `vendor` with `vendor_set` false is C's NULL.
#[derive(Clone, Debug, Default)]
pub(crate) struct VorbisComment {
    /* unlimited user comment fields.  libvorbis writes 'libvorbis'
    whatever vendor is set to in encode */
    pub user_comments: Vec<Vec<u8>>,
    pub comments: i32,
    pub vendor: Vec<u8>,
    pub vendor_set: bool,
}

/* Vorbis ERRORS and return codes ***********************************/

pub(crate) const OV_FALSE: i32 = -1;
pub(crate) const OV_EOF: i32 = -2;
pub(crate) const OV_HOLE: i32 = -3;

pub(crate) const OV_EREAD: i32 = -128;
pub(crate) const OV_EFAULT: i32 = -129;
pub(crate) const OV_EIMPL: i32 = -130;
pub(crate) const OV_EINVAL: i32 = -131;
pub(crate) const OV_ENOTVORBIS: i32 = -132;
pub(crate) const OV_EBADHEADER: i32 = -133;
pub(crate) const OV_EVERSION: i32 = -134;
pub(crate) const OV_ENOTAUDIO: i32 = -135;
pub(crate) const OV_EBADPACKET: i32 = -136;
pub(crate) const OV_EBADLINK: i32 = -137;
pub(crate) const OV_ENOSEEK: i32 = -138;

/* codec_internal.h ****************************************************/

/* (the BLOCKTYPE_* and PACKETBLOBS constants and vorbis_block_internal
are the encoder's) */

/* mode ************************************************************/
/// Translation of `vorbis_info_mode`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VorbisInfoMode {
    pub blockflag: i32,
    pub windowtype: i32,
    pub transformtype: i32,
    pub mapping: i32,
}

/// Translation of `private_state` (synthesis): the encoder's envelope,
/// FFT, psychoacoustics, bitrate manager and header storage aren't
/// kept. C's `transform[2]` arrays of one MDCT lookup each are the two
/// lookups.
#[derive(Debug, Default)]
pub(crate) struct PrivateState {
    /* local lookup storage */
    pub window: [i32; 2],
    pub transform: [MdctLookup; 2], /* block, type */
    pub modebits: i32,
    pub flr: Vec<VorbisLookFloor>,
    pub residue: Vec<res0::VorbisLookResidue0>,

    pub sample_count: i64,
}

/* codec_setup_info contains all the setup information specific to the
specific compression/decompression mode in progress (eg,
psychoacoustic settings, channel setup, options, codebook
etc).
*********************************************************************/

/// Translation of `codec_setup_info` (synthesis: the encoder's
/// psychoacoustic, bitrate and high-level settings aren't kept). C's
/// fixed arrays of pointers are vectors of `Option`s of their sizes; a
/// `None` `fullbooks` is C's NULL.
#[derive(Debug)]
pub(crate) struct CodecSetupInfo {
    /* Vorbis supports only short and long blocks, but allows the
    encoder to choose the sizes */
    pub blocksizes: [i64; 2],

    /* modes are the primary means of supporting on-the-fly different
    blocksizes, different channel mappings (LR or M/A),
    different residue backends, etc.  Each mode consists of a
    blocksize flag and a mapping (along with the mapping setup */
    pub modes: i32,
    pub maps: i32,
    pub floors: i32,
    pub residues: i32,
    pub books: i32,

    pub mode_param: Vec<Option<VorbisInfoMode>>, /* [64] */
    pub map_type: [i32; 64],
    pub map_param: Vec<Option<Box<mapping0::VorbisInfoMapping0>>>, /* [64] */
    pub floor_type: [i32; 64],
    pub floor_param: Vec<Option<VorbisInfoFloor>>, /* [64] */
    pub residue_type: [i32; 64],
    pub residue_param: Vec<Option<Box<res0::VorbisInfoResidue0>>>, /* [64] */
    pub book_param: Vec<Option<Box<StaticCodebook>>>,              /* [256] */
    pub fullbooks: Option<Vec<Codebook>>,

    pub halfrate_flag: i32, /* painless downsample for decode */
}

impl Default for CodecSetupInfo {
    fn default() -> Self {
        CodecSetupInfo {
            blocksizes: [0; 2],
            modes: 0,
            maps: 0,
            floors: 0,
            residues: 0,
            books: 0,
            mode_param: vec![None; 64],
            map_type: [0; 64],
            map_param: (0..64).map(|_| None).collect(),
            floor_type: [0; 64],
            floor_param: (0..64).map(|_| None).collect(),
            residue_type: [0; 64],
            residue_param: (0..64).map(|_| None).collect(),
            book_param: (0..256).map(|_| None).collect(),
            fullbooks: None,
            halfrate_flag: 0,
        }
    }
}

/* backends.h **********************************************************/

/* this would all be simpler/shorter with templates, but.... */
/* Floor backend generic *****************************************/

/// The floor setups (`vorbis_info_floor`, C's `void *` of a
/// `floor_type`).
#[derive(Clone, Debug)]
pub(crate) enum VorbisInfoFloor {
    Floor0(Box<floor0::VorbisInfoFloor0>),
    Floor1(Box<floor1::VorbisInfoFloor1>),
}

/// The floor lookups (`vorbis_look_floor`).
#[derive(Debug)]
pub(crate) enum VorbisLookFloor {
    Floor0(Box<floor0::VorbisLookFloor0>),
    Floor1(Box<floor1::VorbisLookFloor1>),
}

/// What a floor's `inverse1` returns for its `inverse2` (C's `void *`
/// memo in the block's local storage): floor 0's LSP coefficients and
/// amplitude, floor 1's fit values.
#[derive(Debug)]
pub(crate) enum FloorMemo {
    Floor0(Vec<f32>),
    Floor1(Vec<i32>),
}

/* codebook.h **********************************************************/

/* This structure encapsulates huffman and VQ style encoding books; it
doesn't do anything specific to either.

valuelist/quantlist are nonNULL (and q_* significant) only if
there's entry->value mapping to be done.

If encode-side mapping must be done (and thus the entry needs to be
hunted), the auxiliary encode pointer will point to a decision
tree.  This is true of both VQ and huffman, but is mostly useful
with VQ.

*/

/// Translation of `static_codebook`. C's `char` lengths are `i8`.
#[derive(Clone, Debug, Default)]
pub(crate) struct StaticCodebook {
    pub dim: i64,            /* codebook dimensions (elements per vector) */
    pub entries: i64,        /* codebook entries */
    pub lengthlist: Vec<i8>, /* codeword lengths in bits */

    /* mapping ***************************************************************/
    pub maptype: i32, /* 0=none
                      1=implicitly populated values from map column
                      2=listed arbitrary values */

    /* The below does a linear, single monotonic sequence mapping. */
    pub q_min: i64,       /* packed 32 bit float; quant value 0 maps to minval */
    pub q_delta: i64,     /* packed 32 bit float; val 1 - val 0 == delta */
    pub q_quant: i32,     /* bits: 0 < quant <= 16 */
    pub q_sequencep: i32, /* bitflag */

    pub quantlist: Vec<i64>, /* map == 1: (int)(entries^(1/dim)) element column map
                             map == 2: list of dim*entries quantized entry vals
                             */
    pub allocedp: i32,
}

/// Translation of `codebook` (decode). Empty vectors are C's NULLs.
#[derive(Clone, Debug, Default)]
pub(crate) struct Codebook {
    pub dim: i64,          /* codebook dimensions (elements per vector) */
    pub entries: i64,      /* codebook entries */
    pub used_entries: i64, /* populated codebook entries */
    /* (c, the static codebook, is only set by vorbis_book_init_encode()) */

    /* for encode, the below are entry-ordered, fully populated */
    /* for decode, the below are ordered by bitreversed codeword and only
    used entries are populated */
    pub valuelist: Vec<f32>, /* list of dim*entries actual entry values */
    pub codelist: Vec<u32>,  /* list of bitstream codewords for each entry */

    pub dec_index: Vec<i32>, /* only used if sparseness collapsed */
    pub dec_codelengths: Vec<i8>,
    pub dec_firsttable: Vec<u32>,
    pub dec_firsttablen: i32,
    pub dec_maxlength: i32,
    /* (quantvals, minval and delta, for the current encoder's
    centered, integer-only lattice books, are only set by
    vorbis_book_init_encode()) */
}

/* registry.h **********************************************************/

/* (VI_TRANSFORMB: the one transform is the MDCT) */
pub(crate) const VI_WINDOWB: i32 = 1;
pub(crate) const VI_TIMEB: i32 = 1;
pub(crate) const VI_FLOORB: i32 = 2;
pub(crate) const VI_RESB: i32 = 3;
pub(crate) const VI_MAPB: i32 = 1;

/* scales.h ************************************************************/

/// Translation of `fromdB(x)`: `exp((x)*.11512925f)`, with `x` a double.
pub(crate) fn fromdB(x: f64) -> f64 {
    sdl3::stdlib::math::exp(x * 0.11512925f32 as f64)
}

/* The bark scale equations are approximations, since the original
table was somewhat hand rolled.  The below are chosen to have the
best possible fit to the rolled tables, thus their somewhat odd
appearance (these are more accurate and over a longer range than
the oft-quoted bark equations found in the texts I have).  The
approximations are valid from 0 - 30kHz (nyquist) or so.

all f in Hz, z in Bark */

/// Translation of `toBARK(n)` for a `float` `n`: the `float` products
/// promote to `double` at the `atan()` calls and the sum.
pub(crate) fn toBARK(n: f32) -> f64 {
    use sdl3::stdlib::math::atan;
    13.1f32 as f64 * atan((0.00074f32 * n) as f64)
        + 2.24f32 as f64 * atan((n * n * 1.85e-8f32) as f64)
        + (1e-4f32 * n) as f64
}

/* C's conversions *****************************************************/

/// C's conversion of a `double` to `int` on x86-64 (`cvttsd2si`): values
/// out of range (and NaN) give `INT_MIN`.
pub(crate) fn c_int(x: f64) -> i32 {
    if x.is_nan() || x >= 2147483648.0 || x <= -2147483649.0 {
        i32::MIN
    } else {
        x as i32
    }
}

/// C's conversion of a `double` to `long` (64 bits) on x86-64: values
/// out of range (and NaN) give `LONG_MIN`.
pub(crate) fn c_long(x: f64) -> i64 {
    if x.is_nan() || x >= 9223372036854775808.0 || x < -9223372036854775808.0 {
        i64::MIN
    } else {
        x as i64
    }
}

/// `_ogg_malloc()`/`_ogg_calloc()` of `len` elements whose count came
/// from the stream: `None` when the memory can't be had (C's NULL).
pub(crate) fn try_vec<T: Clone + Default>(len: usize) -> Option<Vec<T>> {
    crate::internal::try_alloc(len).ok()
}
