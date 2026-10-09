// Rust translation of src/lib.c and include/dav1d/dav1d.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! dav1d, the AV1 decoder SDL_image's libavif uses.

// (the translation of dav1d keeps upstream's loops over indices, argument
// lists, late initializations, explicit arithmetic and switch layouts as
// they are)
#![allow(
    dead_code,
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::derivable_impls,
    clippy::identity_op,
    clippy::int_plus_one,
    clippy::manual_range_contains,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::precedence,
    clippy::too_many_arguments
)]

mod bitdepth;
mod cdef;
mod cdf;
mod data;
mod dequant_tables;
mod env;
mod getbits;
mod headers;
mod intops;
mod intra_edge;
mod ipred;
mod ipred_prepare;
mod itx;
mod itx_1d;
mod levels;
mod lf_mask;
mod loopfilter;
mod looprestoration;
mod mem;
mod msac;
mod picture;
mod qm;
mod refmvs;
mod scan;
mod tables;
mod warpmv;
mod wedge;
