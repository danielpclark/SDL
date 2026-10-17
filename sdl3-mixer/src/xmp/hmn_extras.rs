// Rust translation of src/hmn_extras.c and src/hmn_extras.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! His Master's Noise synth instruments and MegaArp.

use super::common::*;
use super::effects::FX_MEGAARP;
use super::extras::{ChannelExtras, InstrumentExtras, ModuleExtras};
use super::player::*;
use super::virt::libxmp_virt_setsmp;
use super::*;

/// Translation of `struct hmn_instrument_extras`.
#[derive(Clone, Debug)]
pub(crate) struct HmnInstrumentExtras {
    pub dataloopstart: i32,
    pub dataloopend: i32,
    pub data: [u8; 64],
    pub progvolume: [u8; 64],
}

impl Default for HmnInstrumentExtras {
    fn default() -> Self {
        HmnInstrumentExtras {
            dataloopstart: 0,
            dataloopend: 0,
            data: [0; 64],
            progvolume: [0; 64],
        }
    }
}

/// Translation of `struct hmn_channel_extras`.
#[derive(Clone, Debug, Default)]
pub(crate) struct HmnChannelExtras {
    pub datapos: i32, /* HMN waveform table pointer */
    pub volume: i32,  /* HMN synth volume */
}

#[rustfmt::skip]
static MEGAARP: [[u8; 16]; 16] = [
    [  0,  3,  7, 12, 15, 12,  7,  3,  0,  3,  7, 12, 15, 12,  7,  3 ],
    [  0,  4,  7, 12, 16, 12,  7,  4,  0,  4,  7, 12, 16, 12,  7,  4 ],
    [  0,  3,  8, 12, 15, 12,  8,  3,  0,  3,  8, 12, 15, 12,  8,  3 ],
    [  0,  4,  8, 12, 16, 12,  8,  4,  0,  4,  8, 12, 16, 12,  8,  4 ],
    [  0,  5,  8, 12, 17, 12,  8,  5,  0,  5,  8, 12, 17, 12,  8,  5 ],
    [  0,  5,  9, 12, 17, 12,  9,  5,  0,  5,  9, 12, 17, 12,  9,  5 ],
    [ 12,  0,  7,  0,  3,  0,  7,  0, 12,  0,  7,  0,  3,  0,  7,  0 ],
    [ 12,  0,  7,  0,  4,  0,  7,  0, 12,  0,  7,  0,  4,  0,  7,  0 ],

    [  0,  3,  7,  3,  7, 12,  7, 12, 15, 12,  7, 12,  7,  3,  7,  3 ],
    [  0,  4,  7,  4,  7, 12,  7, 12, 16, 12,  7, 12,  7,  4,  7,  4 ],
    [ 31, 27, 24, 19, 15, 12,  7,  3,  0,  3,  7, 12, 15, 19, 24, 27 ],
    [ 31, 28, 24, 19, 16, 12,  7,  4,  0,  4,  7, 12, 16, 19, 24, 28 ],
    [  0, 12,  0, 12,  0, 12,  0, 12,  0, 12,  0, 12,  0, 12,  0, 12 ],
    [  0, 12, 24, 12,  0, 12, 24, 12,  0, 12, 24, 12,  0, 12, 24, 12 ],
    [  0,  3,  0,  3,  0,  3,  0,  3,  0,  3,  0,  3,  0,  3,  0,  3 ],
    [  0,  4,  0,  4,  0,  4,  0,  4,  0,  4,  0,  4,  0,  4,  0,  4 ],
];

/// Translation of `libxmp_hmn_linear_bend()`.
pub(crate) fn libxmp_hmn_linear_bend(_ctx: &mut ContextData, _chn: usize) -> i32 {
    0
}

/// Translation of `libxmp_hmn_play_extras()`.
pub(crate) fn libxmp_hmn_play_extras(ctx: &mut ContextData, chn: usize) {
    let frame = ctx.p.frame;
    let xc = &mut ctx.p.xc_data[chn];
    let ins = xc.ins as usize;
    let new = xc.test(NEW_NOTE | NEW_INS);
    let ChannelExtras::Hmn(ce) = &mut xc.extra else {
        unreachable!();
    };

    if frame == 0 && new {
        ce.datapos = 0;
    }

    let xxi = &ctx.m.mod_.xxi[ins];
    let InstrumentExtras::Hmn(ie) = &xxi.extra else {
        unreachable!();
    };
    let mut pos = ce.datapos & 63; /* TODO: how are out of bounds values handled? */
    let waveform = ie.data[pos as usize] as i32;
    let volume = (ie.progvolume[pos as usize] & 0x7f) as i32;

    pos += 1;
    if pos > ie.dataloopend {
        pos = ie.dataloopstart;
    }

    ce.datapos = pos;
    ce.volume = volume;

    if waveform < xxi.nsm && xxi.sub[waveform as usize].sid != xc.smp {
        let smp = xxi.sub[waveform as usize].sid;
        xc.smp = smp;
        libxmp_virt_setsmp(ctx, chn as i32, smp);
    }
}

/// Translation of `libxmp_hmn_new_instrument_extras()`.
pub(crate) fn libxmp_hmn_new_instrument_extras(xxi: &mut XmpInstrument) -> i32 {
    xxi.extra = InstrumentExtras::Hmn(Box::default());
    0
}

/// Translation of `libxmp_hmn_new_channel_extras()`.
pub(crate) fn libxmp_hmn_new_channel_extras(xc: &mut ChannelData) -> i32 {
    xc.extra = ChannelExtras::Hmn(Box::default());
    0
}

/// Translation of `libxmp_hmn_reset_channel_extras()`.
pub(crate) fn libxmp_hmn_reset_channel_extras(xc: &mut ChannelData) {
    if let ChannelExtras::Hmn(ce) = &mut xc.extra {
        **ce = HmnChannelExtras::default();
    }
}

/// Translation of `libxmp_hmn_new_module_extras()`.
pub(crate) fn libxmp_hmn_new_module_extras(m: &mut ModuleData) -> i32 {
    m.extra = ModuleExtras::Hmn;
    0
}

/// Translation of `libxmp_hmn_release_module_extras()`.
pub(crate) fn libxmp_hmn_release_module_extras(m: &mut ModuleData) {
    m.extra = ModuleExtras::None;
}

/// Translation of `libxmp_hmn_extras_process_fx()`.
pub(crate) fn libxmp_hmn_extras_process_fx(
    ctx: &mut ContextData,
    chn: usize,
    _note: u8,
    _ins: u8,
    fxt: u8,
    mut fxp: u8,
    _fnum: i32,
) {
    if fxt == FX_MEGAARP {
        /* Not sure if this is correct... */
        fxp = lsn(fxp) as u8;

        let xc = &mut ctx.p.xc_data[chn];
        for i in 0..16 {
            xc.arpeggio.val[i] = MEGAARP[fxp as usize][i] as i8;
        }
        xc.arpeggio.size = 16;
    }
}
