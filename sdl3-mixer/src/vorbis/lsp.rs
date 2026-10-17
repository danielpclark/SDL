// Rust translation of lib/lsp.c (the plain float `vorbis_lsp_to_curve()`)
// from libvorbis 1.3.7 (https://xiph.org/vorbis/, at the revision
// SDL_mixer's external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: LSP (also called LSF) conversion routines
//!
//! The LSP generation code is taken (with minimal modification and a
//! few bugfixes) from "On the Computation of the LSP Frequencies" by
//! Joseph Rothweiler (see <http://www.rothweiler.us> for contact info).
//!
//! Of the three possible LSP to f curve functions (the exact computation
//! (float), a lookup based float implementation, and an integer
//! implementation), the build defines neither `FLOAT_LOOKUP` nor
//! `INT_LOOKUP`, so it has the first: "old, nonoptimized but simple
//! version for any poor sap who needs to figure out what the hell this code
//! does, or wants the other fraction of a dB precision". The
//! LPC-to-LSP conversion (`vorbis_lpc_to_lsp()`, with `cheby()`,
//! `Laguerre_With_Deflation()` and `Newton_Raphson()`) is the encoder's.

use super::fromdB;
use sdl3::stdlib::math::{cos, sqrt};

/* side effect: changes *lsp to cosines of lsp */
/// Translation of `vorbis_lsp_to_curve()`.
pub(crate) fn vorbis_lsp_to_curve(
    curve: &mut [f32],
    map: &[i32],
    n: i32,
    ln: i32,
    lsp: &mut [f32],
    m: i32,
    amp: f32,
    ampoffset: f32,
) {
    let m = m as usize;
    let wdel: f32 = (std::f64::consts::PI / ln as f64) as f32;
    for v in lsp.iter_mut().take(m) {
        *v = (2.0f32 as f64 * cos(*v as f64)) as f32;
    }

    let mut i = 0usize;
    while i < n as usize {
        let mut j: usize;
        let k = map[i];
        let mut p: f32 = 0.5;
        let mut q: f32 = 0.5;
        let w: f32 = (2.0f32 as f64 * cos((wdel * k as f32) as f64)) as f32;
        j = 1;
        while j < m {
            q *= w - lsp[j - 1];
            p *= w - lsp[j];
            j += 2;
        }
        if j == m {
            /* odd order filter; slightly assymetric */
            /* the last coefficient */
            q *= w - lsp[j - 1];
            p *= p * (4.0 - w * w);
            q *= q;
        } else {
            /* even order filter; still symmetric */
            p *= p * (2.0 - w);
            q *= q * (2.0 + w);
        }

        q = fromdB(amp as f64 / sqrt((p + q) as f64) - ampoffset as f64) as f32;

        curve[i] *= q;
        i += 1;
        while map[i] == k {
            curve[i] *= q;
            i += 1;
        }
    }
}
