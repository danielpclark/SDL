// Rust translation of src/extras.c and src/extras.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Format-specific extras of modules, instruments and channels: the MED
//! synth and hold/decay, His Master's Noise synth, Farandole Composer
//! tempo and vibrato, and StarTrekker AM envelopes.
//!
//! Upstream keeps them behind `void *extra` pointers, telling the formats
//! apart by a magic number at their start (`HAS_MED_MODULE_EXTRAS()` and
//! the like); here they're enums.

use super::common::*;
use super::far_extras::*;
use super::flt_extras::*;
use super::hmn_extras::*;
use super::med_extras::*;
use super::player::ChannelData;

/// A module's `extra`.
#[derive(Clone, Debug, Default)]
pub(crate) enum ModuleExtras {
    #[default]
    None,
    Med(Box<MedModuleExtras>),
    Hmn,
    Far(Box<FarModuleExtras>),
    Flt,
}

/// An instrument's `extra`.
#[derive(Clone, Debug, Default)]
pub(crate) enum InstrumentExtras {
    #[default]
    None,
    Med(Box<MedInstrumentExtras>),
    Hmn(Box<HmnInstrumentExtras>),
    Flt(Box<FltInstrumentExtras>),
}

/// A channel's `extra`.
#[derive(Clone, Debug, Default)]
pub(crate) enum ChannelExtras {
    #[default]
    None,
    Med(Box<MedChannelExtras>),
    Hmn(Box<HmnChannelExtras>),
    Far(Box<FarChannelExtras>),
    Flt(Box<FltChannelExtras>),
}

/*
 * Module extras
 */

/// Translation of `libxmp_release_module_extras()`.
pub(crate) fn libxmp_release_module_extras(ctx: &mut ContextData) {
    let m = &mut ctx.m;

    match m.extra {
        ModuleExtras::Med(_) => libxmp_med_release_module_extras(m),
        ModuleExtras::Hmn => libxmp_hmn_release_module_extras(m),
        ModuleExtras::Far(_) => libxmp_far_release_module_extras(m),
        ModuleExtras::Flt => libxmp_flt_release_module_extras(m),
        ModuleExtras::None => {}
    }
}

/*
 * Channel extras
 */

/// Translation of `libxmp_new_channel_extras()`.
pub(crate) fn libxmp_new_channel_extras(ctx: &ContextData, xc: &mut ChannelData) -> i32 {
    let m = &ctx.m;

    match m.extra {
        ModuleExtras::Med(_) => libxmp_med_new_channel_extras(xc),
        ModuleExtras::Hmn => libxmp_hmn_new_channel_extras(xc),
        ModuleExtras::Far(_) => libxmp_far_new_channel_extras(xc),
        ModuleExtras::Flt => libxmp_flt_new_channel_extras(xc),
        ModuleExtras::None => 0,
    }
}

/// Translation of `libxmp_release_channel_extras()`.
pub(crate) fn libxmp_release_channel_extras(ctx: &ContextData, xc: &mut ChannelData) {
    match ctx.m.extra {
        ModuleExtras::None => {}
        _ => xc.extra = ChannelExtras::None,
    }
}

/// Translation of `libxmp_reset_channel_extras()`.
pub(crate) fn libxmp_reset_channel_extras(ctx: &ContextData, xc: &mut ChannelData) {
    match ctx.m.extra {
        ModuleExtras::Med(_) => libxmp_med_reset_channel_extras(xc),
        ModuleExtras::Hmn => libxmp_hmn_reset_channel_extras(xc),
        ModuleExtras::Far(_) => libxmp_far_reset_channel_extras(xc),
        ModuleExtras::Flt => libxmp_flt_reset_channel_extras(xc),
        ModuleExtras::None => {}
    }
}

/*
 * Player extras
 */

/// Translation of `libxmp_play_extras()`.
pub(crate) fn libxmp_play_extras(ctx: &mut ContextData, chn: usize) {
    if matches!(ctx.p.xc_data[chn].extra, ChannelExtras::Far(_)) {
        libxmp_far_play_extras(ctx, chn);
    }

    let ins = ctx.p.xc_data[chn].ins;
    if ins >= ctx.m.mod_.ins {
        /* SFX instruments have no extras */
        return;
    }

    match ctx.m.mod_.xxi[ins as usize].extra {
        InstrumentExtras::Med(_) => libxmp_med_play_extras(ctx, chn),
        InstrumentExtras::Hmn(_) => libxmp_hmn_play_extras(ctx, chn),
        InstrumentExtras::Flt(_) => libxmp_flt_play_extras(ctx, chn),
        InstrumentExtras::None => {}
    }
}

/// The `volume` of a channel's extras (`MED_CHANNEL_EXTRAS(*xc)->volume`
/// and the like), where upstream reads it whatever their type.
fn channel_extras_volume(xc: &ChannelData) -> i32 {
    match &xc.extra {
        ChannelExtras::Med(ce) => ce.volume,
        ChannelExtras::Hmn(ce) => ce.volume,
        ChannelExtras::Flt(ce) => ce.volume,
        // FIXME (upstream): an instrument's extras of another type than
        // its channel's (or none) are read as the channel's: can't happen
        // with the loaders, which give both the same type.
        _ => 0,
    }
}

/// Translation of `libxmp_extras_get_volume()`.
pub(crate) fn libxmp_extras_get_volume(ctx: &ContextData, chn: usize) -> i32 {
    let m = &ctx.m;
    let xc = &ctx.p.xc_data[chn];

    if xc.ins >= m.mod_.ins {
        xc.volume
    } else {
        match m.mod_.xxi[xc.ins as usize].extra {
            InstrumentExtras::Med(_) | InstrumentExtras::Hmn(_) => {
                channel_extras_volume(xc) * xc.volume / 64
            }
            InstrumentExtras::Flt(_) => channel_extras_volume(xc) / 4,
            InstrumentExtras::None => xc.volume,
        }
    }
}

/// Translation of `libxmp_extras_get_period()`.
pub(crate) fn libxmp_extras_get_period(ctx: &mut ContextData, chn: usize) -> i32 {
    if matches!(ctx.p.xc_data[chn].extra, ChannelExtras::Med(_)) {
        libxmp_med_change_period(ctx, chn)
    } else {
        0
    }
}

/// Translation of `libxmp_extras_get_linear_bend()`.
pub(crate) fn libxmp_extras_get_linear_bend(ctx: &mut ContextData, chn: usize) -> i32 {
    match ctx.p.xc_data[chn].extra {
        ChannelExtras::Med(_) => libxmp_med_linear_bend(ctx, chn),
        ChannelExtras::Hmn(_) => libxmp_hmn_linear_bend(ctx, chn),
        _ => 0,
    }
}

/// Translation of `libxmp_extras_process_fx()`.
pub(crate) fn libxmp_extras_process_fx(
    ctx: &mut ContextData,
    chn: usize,
    note: u8,
    ins: u8,
    fxt: u8,
    fxp: u8,
    fnum: i32,
) {
    match ctx.p.xc_data[chn].extra {
        ChannelExtras::Med(_) => libxmp_med_extras_process_fx(ctx, chn, note, ins, fxt, fxp, fnum),
        ChannelExtras::Hmn(_) => libxmp_hmn_extras_process_fx(ctx, chn, note, ins, fxt, fxp, fnum),
        ChannelExtras::Far(_) => libxmp_far_extras_process_fx(ctx, chn, note, ins, fxt, fxp, fnum),
        _ => {}
    }
}
