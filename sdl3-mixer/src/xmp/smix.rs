// Rust translation of parts of src/smix.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The sound effect mixer's instruments and samples, as the player
//! consults them. (Starting it and playing sound effects, `xmp_start_smix()`
//! and the `xmp_smix_*()` functions, aren't translated: SDL_mixer doesn't
//! use them, so `smix.chn`, `smix.ins` and `smix.smp` are always 0.)

use super::common::*;
use super::*;

/// Translation of `libxmp_get_instrument()`.
pub(crate) fn libxmp_get_instrument(ctx: &ContextData, ins: i32) -> Option<&XmpInstrument> {
    let smix = &ctx.smix;
    let mod_ = &ctx.m.mod_;

    if ins < 0 {
        None
    } else if ins < mod_.ins {
        Some(&mod_.xxi[ins as usize])
    } else if ins < mod_.ins + smix.ins {
        Some(&smix.xxi[(ins - mod_.ins) as usize])
    } else {
        None
    }
}

/// Translation of `libxmp_get_sample()`.
pub(crate) fn libxmp_get_sample(ctx: &ContextData, smp: i32) -> Option<&XmpSample> {
    let smix = &ctx.smix;
    let mod_ = &ctx.m.mod_;

    if smp < 0 {
        None
    } else if smp < mod_.smp {
        Some(&mod_.xxs[smp as usize])
    } else if smp < mod_.smp + smix.smp {
        Some(&smix.xxs[(smp - mod_.smp) as usize])
    } else {
        None
    }
}

/// Translation of `xmp_end_smix()`.
pub(crate) fn xmp_end_smix(ctx: &mut ContextData) {
    let smix = &mut ctx.smix;

    if smix.xxs.is_empty() {
        return;
    }

    smix.xxs = Vec::new();
    smix.xxi = Vec::new();
    smix.chn = 0;
    smix.ins = 0;
    smix.smp = 0;
}
