// Rust translation of src/flt_extras.c and src/flt_extras.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! StarTrekker AM instrument extras.
//!
//! Since the envelope overrides channel volume instead of multiplying against
//! it, and some modules (Epsilon (ES)/frankie.mod, WOTW/intro 12.mod) rely
//! on this for some reason, extras are used instead of XM envelopes.
//!
//! TODO: AM vibrato could probably go here.

use super::common::*;
use super::extras::{ChannelExtras, InstrumentExtras, ModuleExtras};
use super::player::*;
use super::*;

/// Translation of `enum flt_env_stage`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FltEnvStage {
    #[default]
    Attack1,
    Attack2,
    Decay,
    Sustain,
    Release,
}

/// Translation of `struct flt_instrument_extras`.
#[derive(Clone, Debug, Default)]
pub(crate) struct FltInstrumentExtras {
    pub l0: u16,
    pub a1l: u16,
    pub a1s: u16,
    pub a2l: u16,
    pub a2s: u16,
    pub sl: u16,
    pub ds: u16,
    pub st: u16,
    pub rs: u16,
    pub p_fall: i16,
    pub fq: u16,
}

/// Translation of `struct flt_channel_extras`.
#[derive(Clone, Debug, Default)]
pub(crate) struct FltChannelExtras {
    pub volume: i32,
    pub sustain: i32,
    pub env_stage: FltEnvStage,
}

/// Translation of `libxmp_flt_tick_envelope()`.
fn libxmp_flt_tick_envelope(
    ce: &mut FltChannelExtras,
    target: i32,
    rate: i32,
    next_stage: FltEnvStage,
) {
    let mut x;

    if target > ce.volume {
        x = ce.volume + rate;
        if x > target {
            x = target;
        }
    } else {
        x = ce.volume - rate;
        if x < target {
            x = target;
        }
    }

    ce.volume = x;
    if x == target {
        ce.env_stage = next_stage;
    }
}

/// Translation of `libxmp_flt_play_extras()`.
pub(crate) fn libxmp_flt_play_extras(ctx: &mut ContextData, chn: usize) {
    let frame = ctx.p.frame;
    let xc = &mut ctx.p.xc_data[chn];
    let InstrumentExtras::Flt(ie) = &ctx.m.mod_.xxi[xc.ins as usize].extra else {
        unreachable!();
    };
    let (new_note, toneporta, new_ins) = (xc.test(NEW_NOTE), xc.test(TONEPORTA), xc.test(NEW_INS));

    if frame == 0 {
        if new_note && !toneporta {
            /* Note with or without ins #, no toneporta -> reset. */
            let ChannelExtras::Flt(ce) = &mut xc.extra else {
                unreachable!();
            };
            ce.volume = ie.l0 as i32;
            ce.sustain = ie.st as i32;
            ce.env_stage = FltEnvStage::Attack1;
        } else if new_ins {
            /* Ins # without note cuts (regardless of # or toneporta).
             * Ins # with note and toneporta also cuts(?!). */
            let ChannelExtras::Flt(ce) = &mut xc.extra else {
                unreachable!();
            };
            ce.volume = 0;
            ce.env_stage = FltEnvStage::Release;
        } else if new_note && toneporta {
            /* StarTrekker forgot to adjust the toneporta target by
             * FQ, so reverse transpose (flt_am_period_fall.mod). */
            xc.porta.target /= 1i32.wrapping_shl(ie.fq as u32) as f64;
            xc.porta.dir = if xc.period < xc.porta.target { 1 } else { -1 };
        }
    }

    let ChannelExtras::Flt(ce) = &mut xc.extra else {
        unreachable!();
    };
    match ce.env_stage {
        FltEnvStage::Attack1 => {
            libxmp_flt_tick_envelope(ce, ie.a1l as i32, ie.a1s as i32, FltEnvStage::Attack2)
        }
        FltEnvStage::Attack2 => {
            libxmp_flt_tick_envelope(ce, ie.a2l as i32, ie.a2s as i32, FltEnvStage::Decay)
        }
        FltEnvStage::Decay => {
            libxmp_flt_tick_envelope(ce, ie.sl as i32, ie.ds as i32, FltEnvStage::Sustain)
        }
        FltEnvStage::Sustain => {
            /* Total duration is ST + 1 ticks. */
            if ce.sustain > 0 {
                ce.sustain -= 1;
            } else {
                ce.env_stage = FltEnvStage::Release;
            }
        }
        FltEnvStage::Release => libxmp_flt_tick_envelope(ce, 0, ie.rs as i32, FltEnvStage::Release),
    }

    /* Add directly to the period--this influences things like toneporta. */
    xc.period += ie.p_fall as f64;
}

/// Translation of `libxmp_flt_new_instrument_extras()`.
pub(crate) fn libxmp_flt_new_instrument_extras(xxi: &mut XmpInstrument) -> i32 {
    xxi.extra = InstrumentExtras::Flt(Box::default());
    0
}

/// Translation of `libxmp_flt_new_channel_extras()`.
pub(crate) fn libxmp_flt_new_channel_extras(xc: &mut ChannelData) -> i32 {
    xc.extra = ChannelExtras::Flt(Box::default());
    0
}

/// Translation of `libxmp_flt_reset_channel_extras()`.
pub(crate) fn libxmp_flt_reset_channel_extras(xc: &mut ChannelData) {
    if let ChannelExtras::Flt(ce) = &mut xc.extra {
        **ce = FltChannelExtras::default();
    }
}

/// Translation of `libxmp_flt_new_module_extras()`.
pub(crate) fn libxmp_flt_new_module_extras(m: &mut ModuleData) -> i32 {
    m.extra = ModuleExtras::Flt;
    0
}

/// Translation of `libxmp_flt_release_module_extras()`.
pub(crate) fn libxmp_flt_release_module_extras(m: &mut ModuleData) {
    m.extra = ModuleExtras::None;
}
