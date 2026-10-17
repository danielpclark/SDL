// Rust translation of src/effects.c and src/effects.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The effects of the pattern events: `libxmp_process_fx()`.

use super::common::*;
use super::extras::libxmp_extras_process_fx;
use super::flow::*;
use super::lfo::*;
use super::period::libxmp_note_to_period;
use super::player::*;
use super::smix::libxmp_get_instrument;
use super::virt::*;
use super::*;

/* Protracker effects */
pub(crate) const FX_ARPEGGIO: u8 = 0x00;
pub(crate) const FX_PORTA_UP: u8 = 0x01;
pub(crate) const FX_PORTA_DN: u8 = 0x02;
pub(crate) const FX_TONEPORTA: u8 = 0x03;
pub(crate) const FX_VIBRATO: u8 = 0x04;
pub(crate) const FX_TONE_VSLIDE: u8 = 0x05;
pub(crate) const FX_VIBRA_VSLIDE: u8 = 0x06;
pub(crate) const FX_TREMOLO: u8 = 0x07;
pub(crate) const FX_OFFSET: u8 = 0x09;
pub(crate) const FX_VOLSLIDE: u8 = 0x0a;
pub(crate) const FX_JUMP: u8 = 0x0b;
pub(crate) const FX_VOLSET: u8 = 0x0c;
pub(crate) const FX_BREAK: u8 = 0x0d;
pub(crate) const FX_EXTENDED: u8 = 0x0e;
pub(crate) const FX_SPEED: u8 = 0x0f;

/* Fast tracker effects */
pub(crate) const FX_SETPAN: u8 = 0x08;

/* Fast Tracker II effects */
pub(crate) const FX_GLOBALVOL: u8 = 0x10;
pub(crate) const FX_GVOL_SLIDE: u8 = 0x11;
pub(crate) const FX_KEYOFF: u8 = 0x14;
pub(crate) const FX_ENVPOS: u8 = 0x15;
pub(crate) const FX_PANSLIDE: u8 = 0x19;
pub(crate) const FX_MULTI_RETRIG: u8 = 0x1b;
pub(crate) const FX_TREMOR: u8 = 0x1d;
pub(crate) const FX_XF_PORTA: u8 = 0x21;

/* Protracker extended effects */
pub(crate) const EX_FILTER: i32 = 0x00;
pub(crate) const EX_F_PORTA_UP: i32 = 0x01;
pub(crate) const EX_F_PORTA_DN: i32 = 0x02;
pub(crate) const EX_GLISS: i32 = 0x03;
pub(crate) const EX_VIBRATO_WF: i32 = 0x04;
pub(crate) const EX_FINETUNE: i32 = 0x05;
pub(crate) const EX_PATTERN_LOOP: i32 = 0x06;
pub(crate) const EX_TREMOLO_WF: i32 = 0x07;
pub(crate) const EX_SETPAN: i32 = 0x08;
pub(crate) const EX_RETRIG: i32 = 0x09;
pub(crate) const EX_F_VSLIDE_UP: i32 = 0x0a;
pub(crate) const EX_F_VSLIDE_DN: i32 = 0x0b;
pub(crate) const EX_CUT: i32 = 0x0c;
pub(crate) const EX_DELAY: i32 = 0x0d;
pub(crate) const EX_PATT_DELAY: i32 = 0x0e;
pub(crate) const EX_INVLOOP: i32 = 0x0f;

/* XM extended effects 2 */
pub(crate) const XX_XF_PORTA_UP: i32 = 0x01;
pub(crate) const XX_XF_PORTA_DN: i32 = 0x02;

/* Oktalyzer effects */
pub(crate) const FX_OKT_ARP3: u8 = 0x70;
pub(crate) const FX_OKT_ARP4: u8 = 0x71;
pub(crate) const FX_OKT_ARP5: u8 = 0x72;
pub(crate) const FX_NSLIDE2_DN: u8 = 0x73;
pub(crate) const FX_NSLIDE2_UP: u8 = 0x74;
pub(crate) const FX_F_NSLIDE_DN: u8 = 0x75;
pub(crate) const FX_F_NSLIDE_UP: u8 = 0x76;

/* Persistent effects -- for FNK */
pub(crate) const FX_PER_PORTA_DN: u8 = 0x78;
pub(crate) const FX_PER_PORTA_UP: u8 = 0x79;
pub(crate) const FX_PER_TPORTA: u8 = 0x7a;
pub(crate) const FX_PER_VIBRATO: u8 = 0x7b;
pub(crate) const FX_PER_VSLD_UP: u8 = 0x7c;
pub(crate) const FX_PER_VSLD_DN: u8 = 0x7d;
pub(crate) const FX_SPEED_CP: u8 = 0x7e;
pub(crate) const FX_PER_CANCEL: u8 = 0x7f;

/* 669 frequency based effects */
pub(crate) const FX_669_PORTA_UP: u8 = 0x60;
pub(crate) const FX_669_PORTA_DN: u8 = 0x61;
pub(crate) const FX_669_TPORTA: u8 = 0x62;
pub(crate) const FX_669_FINETUNE: u8 = 0x63;
pub(crate) const FX_669_VIBRATO: u8 = 0x64;

/* FAR effects */
pub(crate) const FX_FAR_PORTA_UP: u8 = 0x65; /* FAR pitch offset up */
pub(crate) const FX_FAR_PORTA_DN: u8 = 0x66; /* FAR pitch offset down */
pub(crate) const FX_FAR_TPORTA: u8 = 0x67; /* FAR persistent tone portamento */
pub(crate) const FX_FAR_TEMPO: u8 = 0x68; /* FAR coarse tempo and tempo mode */
pub(crate) const FX_FAR_F_TEMPO: u8 = 0x69; /* FAR fine tempo slide up/down */
pub(crate) const FX_FAR_VIBDEPTH: u8 = 0x6a; /* FAR set vibrato depth */
pub(crate) const FX_FAR_VIBRATO: u8 = 0x6b; /* FAR persistent vibrato */
pub(crate) const FX_FAR_SLIDEVOL: u8 = 0x6c; /* FAR persistent slide-to-volume */
pub(crate) const FX_FAR_RETRIG: u8 = 0x6d; /* FAR retrigger */
pub(crate) const FX_FAR_DELAY: u8 = 0x6e; /* FAR note offset */

/* ULT effects */
pub(crate) const FX_ULT_TEMPO: u8 = 0x5f;
pub(crate) const FX_ULT_TPORTA: u8 = 0x6f;

/* IT effects */
pub(crate) const FX_TRK_VOL: u8 = 0x80;
pub(crate) const FX_TRK_VSLIDE: u8 = 0x81;
pub(crate) const FX_TRK_FVSLIDE: u8 = 0x82;
pub(crate) const FX_IT_INSTFUNC: u8 = 0x83;
pub(crate) const FX_FLT_CUTOFF: u8 = 0x84;
pub(crate) const FX_FLT_RESN: u8 = 0x85;
pub(crate) const FX_IT_BPM: u8 = 0x87;
pub(crate) const FX_IT_ROWDELAY: u8 = 0x88;
pub(crate) const FX_IT_PANSLIDE: u8 = 0x89;
pub(crate) const FX_PANBRELLO: u8 = 0x8a;
pub(crate) const FX_PANBRELLO_WF: u8 = 0x8b;
pub(crate) const FX_HIOFFSET: u8 = 0x8c;
pub(crate) const FX_IT_BREAK: u8 = 0x8e; /* like FX_BREAK with hex parameter */
pub(crate) const FX_MACRO_SET: u8 = 0xbd; /* Set active IT parametered MIDI macro */
pub(crate) const FX_MACRO: u8 = 0xbe; /* Execute IT MIDI macro */
pub(crate) const FX_MACROSMOOTH: u8 = 0xbf; /* Execute IT MIDI macro slide */

/* MED effects */
pub(crate) const FX_HOLD_DECAY: u8 = 0x90;
pub(crate) const FX_SETPITCH: u8 = 0x91;
pub(crate) const FX_VIBRATO2: u8 = 0x92;
pub(crate) const FX_MED_RETRIG: u8 = 0x93;

/* IMF effects */
pub(crate) const FX_IMF_FPORTA_DN: u8 = 0x9a; /* IMF 1/16th fine porta down, 1xx/2xx mem */
pub(crate) const FX_IMF_FPORTA_UP: u8 = 0x9b; /* IMF 1/16th fine porta up, 1xx/2xx mem */

/* PTM effects */
pub(crate) const FX_NSLIDE_DN: u8 = 0x9c; /* IMF/PTM note slide down */
pub(crate) const FX_NSLIDE_UP: u8 = 0x9d; /* IMF/PTM note slide up */
pub(crate) const FX_NSLIDE_R_UP: u8 = 0x9e; /* PTM note slide down with retrigger */
pub(crate) const FX_NSLIDE_R_DN: u8 = 0x9f; /* PTM note slide up with retrigger */

/* Extra effects */
pub(crate) const FX_VOLSLIDE_UP: u8 = 0xa0; /* SFX, MDL */
pub(crate) const FX_VOLSLIDE_DN: u8 = 0xa1;
pub(crate) const FX_F_VSLIDE: u8 = 0xa5; /* IMF/MDL */
pub(crate) const FX_CHORUS: u8 = 0xa9; /* IMF */
pub(crate) const FX_ICE_SPEED: u8 = 0xa2;
pub(crate) const FX_REVERB: u8 = 0xaa; /* IMF */
pub(crate) const FX_MED_HOLD: u8 = 0xb1; /* MMD hold/decay */
pub(crate) const FX_MEGAARP: u8 = 0xb2; /* Smaksak effect 7: MegaArp */
pub(crate) const FX_VOL_ADD: u8 = 0xb6; /* SFX change volume up */
pub(crate) const FX_VOL_SUB: u8 = 0xb7; /* SFX change volume down */
pub(crate) const FX_PITCH_ADD: u8 = 0xb8; /* SFX add steps to current note */
pub(crate) const FX_PITCH_SUB: u8 = 0xb9; /* SFX add steps to current note */
pub(crate) const FX_LINE_JUMP: u8 = 0xba; /* Archimedes jump to line in current order */
pub(crate) const FX_RETRIG: u8 = 0xbb; /* Retrigger with extended range (LIQ, DSym) */

pub(crate) const FX_SURROUND: u8 = 0x8d; /* S3M/IT */
pub(crate) const FX_REVERSE: u8 = 0x8f; /* XM/IT/others: play forward/reverse */
pub(crate) const FX_S3M_SPEED: u8 = 0xa3; /* S3M */
pub(crate) const FX_VOLSLIDE_2: u8 = 0xa4;
pub(crate) const FX_FINETUNE: u8 = 0xa6;
pub(crate) const FX_S3M_BPM: u8 = 0xab; /* S3M */
pub(crate) const FX_FINE_VIBRATO: u8 = 0xac; /* S3M/PTM/IMF/LIQ */
pub(crate) const FX_F_VSLIDE_UP: u8 = 0xad; /* MMD */
pub(crate) const FX_F_VSLIDE_DN: u8 = 0xae; /* MMD */
pub(crate) const FX_F_PORTA_UP: u8 = 0xaf; /* MMD */
pub(crate) const FX_F_PORTA_DN: u8 = 0xb0; /* MMD */
pub(crate) const FX_PATT_DELAY: u8 = 0xb3; /* MMD */
pub(crate) const FX_S3M_ARPEGGIO: u8 = 0xb4;
pub(crate) const FX_PANSL_NOMEM: u8 = 0xb5; /* XM volume column */

pub(crate) const FX_VSLIDE_UP_2: u8 = 0xc0; /* IT volume column volume slide */
pub(crate) const FX_VSLIDE_DN_2: u8 = 0xc1;
pub(crate) const FX_F_VSLIDE_UP_2: u8 = 0xc2;
pub(crate) const FX_F_VSLIDE_DN_2: u8 = 0xc3;

/// An effect memory cell (an `int` or a `uint8` field of the channel).
trait MemCell {
    fn load(&self) -> u8;
    fn store(&mut self, v: u8);
}

impl MemCell for i32 {
    fn load(&self) -> u8 {
        *self as u8
    }
    fn store(&mut self, v: u8) {
        *self = v as i32;
    }
}

impl MemCell for u8 {
    fn load(&self) -> u8 {
        *self
    }
    fn store(&mut self, v: u8) {
        *self = v;
    }
}

/// `SET_LFO_NOTZERO(lfo, depth, rate)`
fn set_lfo_notzero(lfo: &mut Lfo, depth: i32, rate: i32) {
    if depth != 0 {
        libxmp_lfo_set_depth(lfo, depth);
    }
    if rate != 0 {
        libxmp_lfo_set_rate(lfo, rate);
    }
}

/// `EFFECT_MEMORY__(p, m)`
fn effect_memory__<M: MemCell>(p: &mut u8, m: &mut M) {
    if *p == 0 {
        *p = m.load();
    } else {
        m.store(*p);
    }
}

/* ST3 effect memory is not a bug, but it's a weird implementation and it's
 * unlikely to be supported in anything other than ST3 (or OpenMPT).
 */
/// `EFFECT_MEMORY(p, m)`: `m` names a field of the channel.
macro_rules! effect_memory {
    ($st3:expr, $xc:expr, $p:expr, $($m:tt)+) => {
        if $st3 {
            effect_memory__(&mut $p, &mut $xc.vol.memory);
        } else {
            effect_memory__(&mut $p, &mut $xc.$($m)+);
        }
    };
}

/// `EFFECT_MEMORY_GET(p, m)`
macro_rules! effect_memory_get {
    ($st3:expr, $xc:expr, $p:expr, $($m:tt)+) => {
        if $st3 {
            $p = $xc.vol.memory;
        } else {
            $p = $xc.$($m)+;
        }
    };
}

/// `EFFECT_MEMORY_SETONLY(p, m)`
macro_rules! effect_memory_setonly {
    ($st3:expr, $xc:expr, $p:expr, $($m:tt)+) => {
        effect_memory__(&mut $p, &mut $xc.$($m)+);
        if $st3 && $p != 0 {
            $xc.vol.memory = $p as i32;
        }
    };
}

/// `EFFECT_MEMORY_S3M(p)`
macro_rules! effect_memory_s3m {
    ($st3:expr, $xc:expr, $p:expr) => {
        if $st3 {
            effect_memory__(&mut $p, &mut $xc.vol.memory);
        }
    };
}

/// Translation of `do_toneporta()`.
fn do_toneporta(ctx: &mut ContextData, chn: usize, mut note: i32) {
    let target = {
        let m = &ctx.m;
        let xc = &ctx.p.xc_data[chn];
        let instrument = &m.mod_.xxi[xc.ins as usize];
        let mut mapped_xpo = 0;
        let mut mapped = 0;

        if is_valid_note(xc.key) {
            mapped = instrument.map[xc.key as usize].ins as i32;
        }

        if mapped >= instrument.nsm {
            mapped = 0;
        }

        let sub = &instrument.sub[mapped as usize];

        if is_valid_note(note - 1) && (xc.ins as u32) < m.mod_.ins as u32 {
            note -= 1;
            if is_valid_note(xc.key_porta) {
                mapped_xpo = instrument.map[xc.key_porta as usize].xpo as i32;
            }
            Some(libxmp_note_to_period(
                ctx,
                note + sub.xpo + mapped_xpo,
                xc.finetune,
                xc.per_adj,
            ))
        } else {
            None
        }
    };
    let xc = &mut ctx.p.xc_data[chn];
    if let Some(target) = target {
        xc.porta.target = target;
    }
    xc.porta.dir = if xc.period < xc.porta.target { 1 } else { -1 };
}

/// The labels `libxmp_process_fx()` jumps to.
#[derive(Clone, Copy)]
enum Label {
    Arpeggio,
    FPortaUp,
    XfPortaUp,
    FPortaDn,
    XfPortaDn,
    Volslide,
    FVslideUp,
    FVslideDn,
    Setpan,
    Retrig,
    PattDelay,
    S3mSpeed,
    S3mBpm,
    Gvolslide,
    TrkFvslide,
}

/// Translation of `libxmp_process_fx()`.
#[allow(clippy::misrefactored_assign_op)] // (upstream's `val &= val & ~0xffff`)
pub(crate) fn libxmp_process_fx(ctx: &mut ContextData, chn: i32, e: &XmpEvent, fnum: i32) {
    let c = chn as usize;
    let st3 = ctx.m.has_quirk(QUIRK_ST3BUGS);
    let mut note: u8;
    let mut fxp: u8;
    let fxt: u8;
    let mut h: i32;
    let mut l: i32;

    /* key_porta is IT only */
    if ctx.m.read_event_type != READ_EVENT_IT {
        let xc = &mut ctx.p.xc_data[c];
        xc.key_porta = xc.key;
    }

    note = e.note;
    if fnum == 0 {
        fxt = e.fxt;
        fxp = e.fxp;
    } else {
        fxt = e.f2t;
        fxp = e.f2p;
    }

    macro_rules! xc {
        () => {
            ctx.p.xc_data[c]
        };
    }
    macro_rules! has_quirk {
        ($q:expr) => {
            ctx.m.has_quirk($q)
        };
    }

    let mut goto = match fxt {
        FX_ARPEGGIO => Some(Label::Arpeggio),
        FX_S3M_ARPEGGIO => {
            effect_memory!(st3, xc!(), fxp, arpeggio.memory);
            Some(Label::Arpeggio)
        }

        FX_OKT_ARP3 => {
            if fxp != 0 {
                let xc = &mut xc!();
                xc.arpeggio.val[0] = -msn(fxp) as i8;
                xc.arpeggio.val[1] = 0;
                xc.arpeggio.val[2] = lsn(fxp) as i8;
                xc.arpeggio.size = 3;
            }
            None
        }
        FX_OKT_ARP4 => {
            if fxp != 0 {
                let xc = &mut xc!();
                xc.arpeggio.val[0] = 0;
                xc.arpeggio.val[1] = lsn(fxp) as i8;
                xc.arpeggio.val[2] = 0;
                xc.arpeggio.val[3] = -msn(fxp) as i8;
                xc.arpeggio.size = 4;
            }
            None
        }
        FX_OKT_ARP5 => {
            if fxp != 0 {
                let xc = &mut xc!();
                xc.arpeggio.val[0] = lsn(fxp) as i8;
                xc.arpeggio.val[1] = lsn(fxp) as i8;
                xc.arpeggio.val[2] = 0;
                xc.arpeggio.size = 3;
            }
            None
        }
        FX_PORTA_UP => 'b: {
            /* Portamento up */
            effect_memory!(st3, xc!(), fxp, freq.memory);

            if has_quirk!(QUIRK_FINEFX) && (fnum == 0 || !has_quirk!(QUIRK_ITVPOR)) {
                match msn(fxp) {
                    0xf => {
                        fxp &= 0x0f;
                        break 'b Some(Label::FPortaUp);
                    }
                    0xe => {
                        fxp &= 0x0f;
                        break 'b Some(Label::XfPortaUp);
                    }
                    _ => {}
                }
            }

            if fxp != 0 {
                let unisld = has_quirk!(QUIRK_UNISLD);
                let xc = &mut xc!();
                xc.set(PITCHBEND);
                xc.freq.slide = -(fxp as i32);
                if unisld {
                    xc.porta.memory = fxp as i32;
                }
            }
            None
        }
        FX_PORTA_DN => 'b: {
            /* Portamento down */
            /* FT2 has separate up and down memory. */
            if has_quirk!(QUIRK_FT2BUGS) {
                effect_memory!(st3, xc!(), fxp, freq.down_memory);
            } else {
                effect_memory!(st3, xc!(), fxp, freq.memory);
            }

            if has_quirk!(QUIRK_FINEFX) && (fnum == 0 || !has_quirk!(QUIRK_ITVPOR)) {
                match msn(fxp) {
                    0xf => {
                        fxp &= 0x0f;
                        break 'b Some(Label::FPortaDn);
                    }
                    0xe => {
                        fxp &= 0x0f;
                        break 'b Some(Label::XfPortaDn);
                    }
                    _ => {}
                }
            }

            if fxp != 0 {
                let unisld = has_quirk!(QUIRK_UNISLD);
                let xc = &mut xc!();
                xc.set(PITCHBEND);
                xc.freq.slide = fxp as i32;
                if unisld {
                    xc.porta.memory = fxp as i32;
                }
            }
            None
        }
        FX_TONEPORTA => 'b: {
            /* Tone portamento */
            effect_memory_setonly!(st3, xc!(), fxp, porta.memory);

            if fxp != 0 {
                let unisld = has_quirk!(QUIRK_UNISLD);
                let xc = &mut xc!();
                if unisld {
                    /* IT compatible Gxx off */
                    xc.freq.memory = fxp as i32;
                }
                xc.porta.slide += fxp as i32;
            }

            if has_quirk!(QUIRK_IGSTPOR) && note == 0 && xc!().porta.dir == 0 {
                break 'b None;
            }

            if !is_valid_instrument(&ctx.m.mod_, xc!().ins) {
                break 'b None;
            }

            do_toneporta(ctx, c, note as i32);

            xc!().set(TONEPORTA);
            None
        }

        FX_VIBRATO => {
            /* Vibrato */
            let xc = &mut xc!();
            effect_memory_setonly!(st3, xc, fxp, vibrato.memory);

            xc.set(VIBRATO);
            set_lfo_notzero(&mut xc.vibrato.lfo, lsn(fxp) << 2, msn(fxp));
            None
        }
        FX_FINE_VIBRATO => {
            /* Fine vibrato */
            let xc = &mut xc!();
            effect_memory_setonly!(st3, xc, fxp, vibrato.memory);

            xc.set(VIBRATO);
            set_lfo_notzero(&mut xc.vibrato.lfo, lsn(fxp), msn(fxp));
            None
        }

        FX_TONE_VSLIDE => 'b: {
            /* Toneporta + vol slide */
            effect_memory_get!(st3, xc!(), l, porta.memory);
            xc!().porta.slide += l;
            if !is_valid_instrument(&ctx.m.mod_, xc!().ins) {
                break 'b None;
            }
            do_toneporta(ctx, c, note as i32);
            xc!().set(TONEPORTA);
            Some(Label::Volslide)
        }
        FX_VIBRA_VSLIDE => {
            /* Vibrato + vol slide */
            xc!().set(VIBRATO);
            Some(Label::Volslide)
        }

        FX_TREMOLO => {
            /* Tremolo */
            let xc = &mut xc!();
            effect_memory!(st3, xc, fxp, tremolo.memory);
            xc.set(TREMOLO);
            set_lfo_notzero(&mut xc.tremolo.lfo, lsn(fxp), msn(fxp));
            None
        }

        FX_SETPAN => {
            /* Set pan */
            if has_quirk!(QUIRK_PROTRACK) {
                None
            } else {
                Some(Label::Setpan)
            }
        }
        FX_OFFSET => {
            /* Set sample offset */
            let ft2 = has_quirk!(QUIRK_FT2BUGS);
            let xc = &mut xc!();
            if ft2 {
                /* FT2: only set memory when offset activates, see
                 * read_event_ft2 (ft2_offset_memory.xm). */
                fxp = if fxp != 0 {
                    fxp
                } else {
                    xc.offset.memory as u8
                };
            } else {
                effect_memory!(st3, xc, fxp, offset.memory);
            }
            xc.set(OFFSET);
            if note != 0 {
                xc.offset.val &= xc.offset.val & !0xffff;
                xc.offset.val |= (fxp as i32) << 8;
                xc.offset.val2 = (fxp as i32) << 8;
            }
            if e.ins != 0 {
                xc.offset.val2 = (fxp as i32) << 8;
            }
            None
        }
        FX_VOLSLIDE => Some(Label::Volslide),
        FX_VOLSLIDE_2 => {
            /* Secondary volume slide (no memory) */
            if fxp != 0 {
                let xc = &mut xc!();
                xc.set(VOL_SLIDE_2);
                h = msn(fxp);
                l = lsn(fxp);
                xc.vol.slide2 = if h != 0 { h } else { -l };
            }
            None
        }
        FX_JUMP => {
            /* Order jump */
            libxmp_process_pattern_jump(&ctx.m, &mut ctx.p.flow, fxp as i32);
            None
        }
        FX_VOLSET => {
            /* Volume set */
            let xc = &mut xc!();
            xc.set(NEW_VOL);
            xc.volume = fxp as i32;
            if xc.split != 0 {
                let (pair, volume) = (xc.pair as usize, xc.volume);
                ctx.p.xc_data[pair].volume = volume;
            }
            None
        }
        FX_BREAK => {
            /* Pattern break */
            libxmp_process_pattern_break(&ctx.m, &mut ctx.p.flow, 10 * msn(fxp) + lsn(fxp));
            None
        }
        FX_EXTENDED => 'b: {
            /* Extended effect */
            effect_memory_s3m!(st3, xc!(), fxp);
            let fxt = (fxp >> 4) as i32;
            fxp &= 0x0f;
            match fxt {
                EX_FILTER => {
                    /* Amiga led filter */
                    if ctx.m.is_amiga_mod() {
                        ctx.p.filter = (fxp & 1 == 0) as i32;
                    }
                }
                EX_F_PORTA_UP => {
                    /* Fine portamento up */
                    effect_memory!(st3, xc!(), fxp, fine_porta.up_memory);
                    break 'b Some(Label::FPortaUp);
                }
                EX_F_PORTA_DN => {
                    /* Fine portamento down */
                    effect_memory!(st3, xc!(), fxp, fine_porta.down_memory);
                    break 'b Some(Label::FPortaDn);
                }
                EX_GLISS => {
                    /* Glissando toggle */
                    if fxp != 0 {
                        xc!().set_note(NOTE_GLISSANDO);
                    } else {
                        xc!().reset_note(NOTE_GLISSANDO);
                    }
                }
                EX_VIBRATO_WF => {
                    /* Set vibrato waveform */
                    fxp &= 3;
                    libxmp_lfo_set_waveform(&mut xc!().vibrato.lfo, fxp as i32);
                }
                EX_FINETUNE => {
                    /* Set finetune */
                    if !has_quirk!(QUIRK_FT2BUGS) || note > 0 {
                        xc!().finetune = ((fxp as i32) << 4) as i8 as i32;
                    }
                }
                EX_PATTERN_LOOP => {
                    /* Loop pattern */
                    let row = ctx.p.row;
                    libxmp_process_pattern_loop(&ctx.m, &mut ctx.p.flow, chn, row, fxp as i32);
                }
                EX_TREMOLO_WF => {
                    /* Set tremolo waveform */
                    libxmp_lfo_set_waveform(&mut xc!().tremolo.lfo, (fxp & 3) as i32);
                }
                EX_SETPAN => {
                    fxp <<= 4;
                    break 'b Some(Label::Setpan);
                }
                EX_RETRIG => {
                    /* Retrig note */
                    break 'b Some(Label::Retrig);
                }
                EX_F_VSLIDE_UP => {
                    /* Fine volume slide up */
                    effect_memory!(st3, xc!(), fxp, fine_vol.up_memory);
                    break 'b Some(Label::FVslideUp);
                }
                EX_F_VSLIDE_DN => {
                    /* Fine volume slide down */
                    effect_memory!(st3, xc!(), fxp, fine_vol.down_memory);
                    break 'b Some(Label::FVslideDn);
                }
                EX_CUT => {
                    /* Cut note */
                    let xc = &mut xc!();
                    xc.set(RETRIG);
                    xc.set_note(NOTE_CUT); /* for IT cut-carry */
                    xc.retrig.val = fxp as i32 + 1;
                    xc.retrig.count = xc.retrig.val;
                    xc.retrig.type_ = 0x10;
                }
                EX_DELAY => {
                    /* Note delay */
                    /* computed at frame loop */
                }
                EX_PATT_DELAY => {
                    /* Pattern delay */
                    break 'b Some(Label::PattDelay);
                }
                EX_INVLOOP => {
                    /* Invert loop / funk repeat */
                    xc!().invloop.speed = fxp as i32;
                }
                _ => {}
            }
            None
        }
        FX_SPEED => {
            /* Set speed */
            if has_quirk!(QUIRK_NOBPM) || ctx.p.flags & XMP_FLAGS_VBLANK != 0 {
                Some(Label::S3mSpeed)
            } else if fxp < 0x20 {
                /* speedup.xm needs BPM = 20 */
                Some(Label::S3mSpeed)
            } else {
                Some(Label::S3mBpm)
            }
        }

        FX_FINETUNE => {
            xc!().finetune = fxp as i32 - 0x80;
            None
        }

        FX_F_VSLIDE_UP => {
            /* Fine volume slide up */
            effect_memory!(st3, xc!(), fxp, fine_vol.up_memory);
            Some(Label::FVslideUp)
        }
        FX_F_VSLIDE_DN => {
            /* Fine volume slide down */
            effect_memory!(st3, xc!(), fxp, fine_vol.up_memory);
            Some(Label::FVslideDn)
        }

        FX_F_PORTA_UP => Some(Label::FPortaUp),
        FX_F_PORTA_DN => Some(Label::FPortaDn),
        FX_PATT_DELAY => Some(Label::PattDelay),

        FX_S3M_SPEED => {
            /* Set S3M speed */
            effect_memory_s3m!(st3, xc!(), fxp);
            Some(Label::S3mSpeed)
        }
        FX_S3M_BPM => Some(Label::S3mBpm),

        FX_IT_BPM => {
            /* Set IT BPM */
            if msn(fxp) == 0 {
                let xc = &mut xc!();
                xc.set(TEMPO_SLIDE);
                if lsn(fxp) != 0 {
                    /* T0x - Tempo slide down by x */
                    xc.tempo_slide = -lsn(fxp);
                }
                /* T00 - Repeat previous slide */
            } else if msn(fxp) == 1 {
                /* T1x - Tempo slide up by x */
                let xc = &mut xc!();
                xc.set(TEMPO_SLIDE);
                xc.tempo_slide = lsn(fxp);
            } else {
                if (fxp as i32) < XMP_MIN_BPM {
                    fxp = XMP_MIN_BPM as u8;
                }
                ctx.p.bpm = fxp as i32;
            }
            None
        }
        FX_IT_ROWDELAY => {
            let f = &mut ctx.p.flow;
            if f.rowdelay_set == 0 {
                f.rowdelay = fxp as i32;
                f.rowdelay_set = ROWDELAY_ON | ROWDELAY_FIRST_FRAME;
            }
            None
        }

        /* From the OpenMPT VolColMemory.it test case:
         * "Volume column commands a, b, c and d (volume slide) share one
         *  effect memory, but it should not be shared with Dxy in the effect
         *  column.
         */
        FX_VSLIDE_UP_2 => {
            /* Fine volume slide up */
            let xc = &mut xc!();
            effect_memory!(st3, xc, fxp, vol.memory2);
            xc.set(VOL_SLIDE_2);
            xc.vol.slide2 = fxp as i32;
            None
        }
        FX_VSLIDE_DN_2 => {
            /* Fine volume slide down */
            let xc = &mut xc!();
            effect_memory!(st3, xc, fxp, vol.memory2);
            xc.set(VOL_SLIDE_2);
            xc.vol.slide2 = -(fxp as i32);
            None
        }
        FX_F_VSLIDE_UP_2 => {
            /* Fine volume slide up */
            let xc = &mut xc!();
            effect_memory!(st3, xc, fxp, vol.memory2);
            xc.set(FINE_VOLS_2);
            xc.vol.fslide2 = fxp as i32;
            None
        }
        FX_F_VSLIDE_DN_2 => {
            /* Fine volume slide down */
            let xc = &mut xc!();
            effect_memory!(st3, xc, fxp, vol.memory2);
            xc.set(FINE_VOLS_2);
            xc.vol.fslide2 = -(fxp as i32);
            None
        }
        FX_IT_BREAK => {
            /* Pattern break with hex parameter */
            libxmp_process_pattern_break(&ctx.m, &mut ctx.p.flow, fxp as i32);
            None
        }

        FX_GLOBALVOL => {
            /* Set global volume */
            if fxp as i32 > ctx.m.gvolbase {
                ctx.p.gvol = ctx.m.gvolbase;
            } else {
                ctx.p.gvol = fxp as i32;
            }
            None
        }
        FX_GVOL_SLIDE => Some(Label::Gvolslide),
        FX_KEYOFF => {
            /* Key off */
            xc!().keyoff = fxp as i32 + 1;
            None
        }
        FX_ENVPOS => {
            /* Set envelope position */
            /* From OpenMPT SetEnvPos.xm:
             * "When using the Lxx effect, Fasttracker 2 only sets the
             *  panning envelope position if the volume envelope’s sustain
             *  flag is set.
             */
            if has_quirk!(QUIRK_FT2BUGS) {
                let sus = match libxmp_get_instrument(ctx, xc!().ins) {
                    Some(instrument) => instrument.aei.flg & XMP_ENVELOPE_SUS != 0,
                    None => false,
                };
                if sus {
                    xc!().p_idx = fxp as i32;
                }
            } else {
                xc!().p_idx = fxp as i32;
            }
            let xc = &mut xc!();
            xc.v_idx = fxp as i32;
            xc.f_idx = fxp as i32;
            None
        }
        FX_PANSLIDE => {
            /* Pan slide (XM) */
            let xc = &mut xc!();
            effect_memory!(st3, xc, fxp, pan.memory);
            xc.set(PAN_SLIDE);
            xc.pan.slide = lsn(fxp) - msn(fxp);
            None
        }
        FX_PANSL_NOMEM => {
            /* Pan slide (XM volume column) */
            let xc = &mut xc!();
            xc.set(PAN_SLIDE);
            xc.pan.slide = lsn(fxp) - msn(fxp);
            None
        }

        FX_IT_PANSLIDE => {
            /* Pan slide w/ fine pan (IT) */
            let xc = &mut xc!();
            xc.set(PAN_SLIDE);
            if fxp != 0 {
                if msn(fxp) == 0xf {
                    xc.pan.slide = 0;
                    xc.pan.fslide = lsn(fxp);
                } else if lsn(fxp) == 0xf {
                    xc.pan.slide = 0;
                    xc.pan.fslide = -msn(fxp);
                } else {
                    xc.set(PAN_SLIDE);
                    xc.pan.slide = lsn(fxp) - msn(fxp);
                    xc.pan.fslide = 0;
                }
            }
            None
        }

        FX_MULTI_RETRIG => {
            /* Multi retrig */
            let xc = &mut xc!();
            effect_memory_s3m!(st3, xc, fxp);
            if fxp != 0 {
                xc.retrig.val = lsn(fxp);
                xc.retrig.type_ = msn(fxp);
            }
            if note != 0 {
                xc.retrig.count = xc.retrig.val + 1;
            }
            xc.retrig.limit = 0;
            xc.set(RETRIG);
            None
        }
        FX_TREMOR => {
            /* Tremor */
            let ft2 = ctx.m.is_player_mode_ft2();
            let xc = &mut xc!();
            effect_memory!(st3, xc, fxp, tremor.memory);
            xc.tremor.up = msn(fxp) as u8;
            xc.tremor.down = lsn(fxp) as u8;
            if !ft2 {
                if xc.tremor.up == 0 {
                    xc.tremor.up += 1;
                }
                if xc.tremor.down == 0 {
                    xc.tremor.down += 1;
                }
            }
            xc.set(TREMOR);
            None
        }
        FX_XF_PORTA => {
            /* Extra fine portamento */
            h = msn(fxp);
            fxp &= 0x0f;
            match h {
                XX_XF_PORTA_UP => {
                    /* Extra fine portamento up */
                    effect_memory!(st3, xc!(), fxp, fine_porta.xf_up_memory);
                    Some(Label::XfPortaUp)
                }
                XX_XF_PORTA_DN => {
                    /* Extra fine portamento down */
                    effect_memory!(st3, xc!(), fxp, fine_porta.xf_down_memory);
                    Some(Label::XfPortaDn)
                }
                _ => None,
            }
        }
        FX_SURROUND => {
            xc!().pan.surround = fxp as i32;
            None
        }
        FX_REVERSE => {
            /* Play forward/backward */
            libxmp_virt_reverse(ctx, chn, fxp as i32);
            None
        }

        FX_TRK_VOL => {
            /* Track volume setting */
            if fxp as i32 <= ctx.m.volbase {
                xc!().mastervol = fxp as i32;
            }
            None
        }
        FX_TRK_VSLIDE => 'b: {
            /* Track volume slide */
            let finefx = has_quirk!(QUIRK_FINEFX);
            let volpdn = has_quirk!(QUIRK_VOLPDN);
            let xc = &mut xc!();
            if fxp == 0 {
                fxp = xc.trackvol.memory as u8;
                if fxp == 0 {
                    break 'b None;
                }
            }

            if finefx {
                h = msn(fxp);
                l = lsn(fxp);
                if h == 0xf && l != 0 {
                    xc.trackvol.memory = fxp as i32;
                    fxp &= 0x0f;
                    break 'b Some(Label::TrkFvslide);
                } else if l == 0xf && h != 0 {
                    xc.trackvol.memory = fxp as i32;
                    fxp &= 0xf0;
                    break 'b Some(Label::TrkFvslide);
                }
            }

            xc.set(TRK_VSLIDE);
            if fxp != 0 {
                h = msn(fxp);
                l = lsn(fxp);

                xc.trackvol.memory = fxp as i32;
                if volpdn {
                    xc.trackvol.slide = if l != 0 { -l } else { h };
                } else {
                    xc.trackvol.slide = if h != 0 { h } else { -l };
                }
            }

            None
        }
        FX_TRK_FVSLIDE => Some(Label::TrkFvslide),

        FX_IT_INSTFUNC => {
            match fxp {
                0 => {
                    /* Past note cut */
                    libxmp_virt_pastnote(ctx, chn, VIRT_ACTION_CUT);
                }
                1 => {
                    /* Past note off */
                    libxmp_virt_pastnote(ctx, chn, VIRT_ACTION_OFF);
                }
                2 => {
                    /* Past note fade */
                    libxmp_virt_pastnote(ctx, chn, VIRT_ACTION_FADE);
                }
                3 => {
                    /* Set NNA to note cut */
                    libxmp_virt_setnna(ctx, chn, XMP_INST_NNA_CUT);
                }
                4 => {
                    /* Set NNA to continue */
                    libxmp_virt_setnna(ctx, chn, XMP_INST_NNA_CONT);
                }
                5 => {
                    /* Set NNA to note off */
                    libxmp_virt_setnna(ctx, chn, XMP_INST_NNA_OFF);
                }
                6 => {
                    /* Set NNA to note fade */
                    libxmp_virt_setnna(ctx, chn, XMP_INST_NNA_FADE);
                }
                7 => {
                    /* Turn off volume envelope */
                    xc!().set_per(VENV_PAUSE);
                }
                8 => {
                    /* Turn on volume envelope */
                    xc!().reset_per(VENV_PAUSE);
                }
                9 => {
                    /* Turn off pan envelope */
                    xc!().set_per(PENV_PAUSE);
                }
                0xa => {
                    /* Turn on pan envelope */
                    xc!().reset_per(PENV_PAUSE);
                }
                0xb => {
                    /* Turn off pitch envelope */
                    xc!().set_per(FENV_PAUSE);
                }
                0xc => {
                    /* Turn on pitch envelope */
                    xc!().reset_per(FENV_PAUSE);
                }
                _ => {}
            }
            None
        }
        FX_FLT_CUTOFF => {
            xc!().filter.cutoff = fxp as i32;
            None
        }
        FX_FLT_RESN => {
            xc!().filter.resonance = fxp as i32;
            None
        }
        FX_MACRO_SET => {
            xc!().macro_.active = lsn(fxp);
            None
        }
        FX_MACRO => {
            let xc = &mut xc!();
            xc.set(MIDI_MACRO);
            xc.macro_.val = fxp as f32;
            xc.macro_.slide = 0.0;
            None
        }
        FX_MACROSMOOTH => {
            let speed = ctx.p.speed;
            let xc = &mut xc!();
            if speed != 0 && xc.macro_.val < 0x80 as f32 {
                xc.set(MIDI_MACRO);
                xc.macro_.target = fxp as f32;
                xc.macro_.slide = (fxp as f32 - xc.macro_.val) / speed as f32;
            }
            None
        }
        FX_PANBRELLO => {
            /* Panbrello */
            let xc = &mut xc!();
            xc.set(PANBRELLO);
            set_lfo_notzero(&mut xc.panbrello.lfo, lsn(fxp) << 4, msn(fxp));
            None
        }
        FX_PANBRELLO_WF => {
            /* Panbrello waveform */
            libxmp_lfo_set_waveform(&mut xc!().panbrello.lfo, (fxp & 3) as i32);
            None
        }
        FX_HIOFFSET => {
            /* High offset */
            let xc = &mut xc!();
            xc.offset.val &= 0xffff;
            xc.offset.val |= (fxp as i32) << 16;
            None
        }

        /* SFX effects */
        FX_VOL_ADD => {
            if is_valid_instrument(&ctx.m.mod_, xc!().ins) {
                let base = ctx.m.mod_.xxi[xc!().ins as usize].sub[0].vol;
                let volbase = ctx.m.volbase;
                let xc = &mut xc!();
                xc.set(NEW_VOL);
                xc.volume = base + fxp as i32;
                if xc.volume > volbase {
                    xc.volume = volbase;
                }
            }
            None
        }
        FX_VOL_SUB => {
            if is_valid_instrument(&ctx.m.mod_, xc!().ins) {
                let base = ctx.m.mod_.xxi[xc!().ins as usize].sub[0].vol;
                let xc = &mut xc!();
                xc.set(NEW_VOL);
                xc.volume = base - fxp as i32;
                if xc.volume < 0 {
                    xc.volume = 0;
                }
            }
            None
        }
        FX_PITCH_ADD => {
            let target = libxmp_note_to_period(ctx, note as i32 - 1, xc!().finetune, 0.0);
            let xc = &mut xc!();
            xc.set_per(TONEPORTA);
            xc.porta.target = target + fxp as f64;
            xc.porta.slide = 2;
            xc.porta.dir = 1;
            None
        }
        FX_PITCH_SUB => {
            let target = libxmp_note_to_period(ctx, note as i32 - 1, xc!().finetune, 0.0);
            let xc = &mut xc!();
            xc.set_per(TONEPORTA);
            xc.porta.target = target - fxp as f64;
            xc.porta.slide = 2;
            xc.porta.dir = -1;
            None
        }

        /* Saga Musix says:
         *
         * "When both nibbles of an Fxx command are set, SoundTracker 2.6
         * applies the both values alternatingly, first the high nibble,
         * then the low nibble on the next row, then the high nibble again...
         * If only the high nibble is set, it should act like if only the low
         * nibble is set (i.e. F30 is the same as F03).
         */
        FX_ICE_SPEED => {
            if fxp != 0 {
                if lsn(fxp) != 0 {
                    ctx.p.st26_speed = (msn(fxp) << 8) | lsn(fxp);
                } else {
                    ctx.p.st26_speed = msn(fxp);
                }
            }
            None
        }

        FX_VOLSLIDE_UP => 'b: {
            /* Vol slide with uint8 arg */
            if has_quirk!(QUIRK_FINEFX) {
                h = msn(fxp);
                l = lsn(fxp);
                if h == 0xf && l != 0 {
                    fxp &= 0x0f;
                    break 'b Some(Label::FVslideUp);
                }
            }

            let xc = &mut xc!();
            if fxp != 0 {
                xc.vol.slide = fxp as i32;
            }
            xc.set(VOL_SLIDE);
            None
        }
        FX_VOLSLIDE_DN => 'b: {
            /* Vol slide with uint8 arg */
            if has_quirk!(QUIRK_FINEFX) {
                h = msn(fxp);
                l = lsn(fxp);
                if h == 0xf && l != 0 {
                    fxp &= 0x0f;
                    break 'b Some(Label::FVslideDn);
                }
            }

            let xc = &mut xc!();
            if fxp != 0 {
                xc.vol.slide = -(fxp as i32);
            }
            xc.set(VOL_SLIDE);
            None
        }
        FX_F_VSLIDE => {
            /* Fine volume slide */
            let xc = &mut xc!();
            xc.set(FINE_VOLS);
            if fxp != 0 {
                h = msn(fxp);
                l = lsn(fxp);
                xc.vol.fslide = if h != 0 { h } else { -l };
            }
            None
        }
        FX_IMF_FPORTA_DN => {
            /* IMF 1/16th precision fine slide down */
            let xc = &mut xc!();
            effect_memory!(st3, xc, fxp, freq.memory);
            if fxp != 0 {
                xc.set(FINE_BEND);
                xc.freq.fslide = 0.0625 * fxp as f64;
            }
            None
        }
        FX_IMF_FPORTA_UP => {
            /* IMF 1/16th precision fine slide up */
            let xc = &mut xc!();
            effect_memory!(st3, xc, fxp, freq.memory);
            if fxp != 0 {
                xc.set(FINE_BEND);
                xc.freq.fslide = -0.0625 * fxp as f64;
            }
            None
        }
        FX_NSLIDE_DN | FX_NSLIDE_UP | FX_NSLIDE_R_DN | FX_NSLIDE_R_UP => {
            let xc = &mut xc!();
            if fxp != 0 {
                if fxt == FX_NSLIDE_R_DN || fxt == FX_NSLIDE_R_UP {
                    xc.retrig.val = msn(fxp);
                    xc.retrig.count = msn(fxp) + 1;
                    xc.retrig.type_ = 0;
                    xc.retrig.limit = 0;
                }

                if fxt == FX_NSLIDE_UP || fxt == FX_NSLIDE_R_UP {
                    xc.noteslide.slide = lsn(fxp);
                } else {
                    xc.noteslide.slide = -lsn(fxp);
                }

                xc.noteslide.speed = msn(fxp);
                xc.noteslide.count = xc.noteslide.speed;
            }
            if fxt == FX_NSLIDE_R_DN || fxt == FX_NSLIDE_R_UP {
                xc.set(RETRIG);
            }
            xc.set(NOTE_SLIDE);
            None
        }
        FX_NSLIDE2_DN => {
            let xc = &mut xc!();
            xc.set(NOTE_SLIDE);
            xc.noteslide.slide = -(fxp as i32);
            xc.noteslide.speed = 1;
            xc.noteslide.count = 1;
            None
        }
        FX_NSLIDE2_UP => {
            let xc = &mut xc!();
            xc.set(NOTE_SLIDE);
            xc.noteslide.slide = fxp as i32;
            xc.noteslide.speed = 1;
            xc.noteslide.count = 1;
            None
        }
        FX_F_NSLIDE_DN => {
            let xc = &mut xc!();
            xc.set(FINE_NSLIDE);
            xc.noteslide.fslide = -(fxp as i32);
            None
        }
        FX_F_NSLIDE_UP => {
            let xc = &mut xc!();
            xc.set(FINE_NSLIDE);
            xc.noteslide.fslide = fxp as i32;
            None
        }

        FX_PER_VIBRATO => {
            /* Persistent vibrato */
            let xc = &mut xc!();
            if lsn(fxp) != 0 {
                xc.set_per(VIBRATO);
            } else {
                xc.reset_per(VIBRATO);
            }
            set_lfo_notzero(&mut xc.vibrato.lfo, lsn(fxp) << 2, msn(fxp));
            None
        }
        FX_PER_PORTA_UP => {
            /* Persistent portamento up */
            let xc = &mut xc!();
            xc.set_per(PITCHBEND);
            xc.freq.slide = -(fxp as i32);
            xc.freq.memory = fxp as i32;
            if xc.freq.memory == 0 {
                xc.reset_per(PITCHBEND);
            }
            None
        }
        FX_PER_PORTA_DN => {
            /* Persistent portamento down */
            let xc = &mut xc!();
            xc.set_per(PITCHBEND);
            xc.freq.slide = fxp as i32;
            xc.freq.memory = fxp as i32;
            if xc.freq.memory == 0 {
                xc.reset_per(PITCHBEND);
            }
            None
        }
        FX_PER_TPORTA => {
            /* Persistent tone portamento */
            if is_valid_instrument(&ctx.m.mod_, xc!().ins) {
                xc!().set_per(TONEPORTA);
                do_toneporta(ctx, c, note as i32);
                let xc = &mut xc!();
                xc.porta.slide = fxp as i32;
                if fxp == 0 {
                    xc.reset_per(TONEPORTA);
                }
            }
            None
        }
        FX_PER_VSLD_UP => {
            /* Persistent volslide up */
            let xc = &mut xc!();
            xc.set_per(VOL_SLIDE);
            xc.vol.slide = fxp as i32;
            if fxp == 0 {
                xc.reset_per(VOL_SLIDE);
            }
            None
        }
        FX_PER_VSLD_DN => {
            /* Persistent volslide down */
            let xc = &mut xc!();
            xc.set_per(VOL_SLIDE);
            xc.vol.slide = -(fxp as i32);
            if fxp == 0 {
                xc.reset_per(VOL_SLIDE);
            }
            None
        }
        FX_VIBRATO2 => {
            /* Deep vibrato (2x) */
            let xc = &mut xc!();
            xc.set(VIBRATO);
            set_lfo_notzero(&mut xc.vibrato.lfo, lsn(fxp) << 3, msn(fxp));
            None
        }
        FX_MED_RETRIG => {
            /* MED 1Fxy delay x, then retrig every y */
            /* initial delay is computed at frame loop */
            let xc = &mut xc!();
            xc.set(RETRIG);
            xc.retrig.val = lsn(fxp);
            xc.retrig.count = lsn(fxp) + 1;
            xc.retrig.type_ = 0;
            xc.retrig.limit = 0;
            None
        }
        FX_SPEED_CP | FX_PER_CANCEL => {
            if fxt == FX_SPEED_CP {
                /* Set speed and ... */
                if fxp != 0 {
                    ctx.p.speed = fxp as i32;
                    ctx.p.st26_speed = 0;
                }
                /* fall through */
            }
            /* Cancel persistent effects */
            xc!().per_flags = 0;
            None
        }

        /* 669 effects */
        FX_669_PORTA_UP => {
            /* 669 portamento up */
            let xc = &mut xc!();
            xc.set_per(PITCHBEND);
            xc.freq.slide = 80 * fxp as i32;
            xc.freq.memory = fxp as i32;
            if xc.freq.memory == 0 {
                xc.reset_per(PITCHBEND);
            }
            None
        }
        FX_669_PORTA_DN => {
            /* 669 portamento down */
            let xc = &mut xc!();
            xc.set_per(PITCHBEND);
            xc.freq.slide = -80 * fxp as i32;
            xc.freq.memory = fxp as i32;
            if xc.freq.memory == 0 {
                xc.reset_per(PITCHBEND);
            }
            None
        }
        FX_669_TPORTA => {
            /* 669 tone portamento */
            if is_valid_instrument(&ctx.m.mod_, xc!().ins) {
                xc!().set_per(TONEPORTA);
                do_toneporta(ctx, c, note as i32);
                let xc = &mut xc!();
                xc.porta.slide = 40 * fxp as i32;
                if fxp == 0 {
                    xc.reset_per(TONEPORTA);
                }
            }
            None
        }
        FX_669_FINETUNE => {
            /* 669 finetune */
            xc!().finetune = 80 * fxp as i8 as i32;
            None
        }
        FX_669_VIBRATO => {
            /* 669 vibrato */
            let xc = &mut xc!();
            if lsn(fxp) != 0 {
                libxmp_lfo_set_waveform(&mut xc.vibrato.lfo, 669);
                xc.set_per(VIBRATO);
            } else {
                xc.reset_per(VIBRATO);
            }
            set_lfo_notzero(&mut xc.vibrato.lfo, 669, 1);
            None
        }

        /* ULT effects */
        FX_ULT_TEMPO => {
            /* ULT tempo */
            /* Has unusual semantics and is hard to split into multiple
             * effects, due to ULT's two effects lanes per channel:
             *
             * 00:    reset both speed and BPM to the default 6/125.
             * 01-2f: set speed
             * 30-ff: set BPM (CIA compatible)
             */
            if fxp == 0 {
                ctx.p.speed = 6;
                ctx.p.st26_speed = 0;
                fxp = 125;
                Some(Label::S3mBpm)
            } else if fxp < 0x30 {
                Some(Label::S3mSpeed)
            } else {
                Some(Label::S3mBpm)
            }
        }
        FX_ULT_TPORTA => {
            /* ULT tone portamento */
            /* Like normal persistent tone portamento, except:
             *
             * 1) Despite the documentation claiming 300 cancels tone
             * portamento, it actually reuses the last parameter.
             *
             * 2) A 3xx without a note will reuse the last target note.
             */
            if is_valid_instrument(&ctx.m.mod_, xc!().ins) {
                let xc = &mut xc!();
                xc.set_per(TONEPORTA);
                effect_memory!(st3, xc, fxp, porta.memory);
                effect_memory!(st3, xc, note, porta.note_memory);
                do_toneporta(ctx, c, note as i32);
                let xc = &mut xc!();
                xc.porta.slide = fxp as i32;
                if fxp == 0 {
                    xc.reset_per(TONEPORTA);
                }
            }
            None
        }

        /* Archimedes (!Tracker, Digital Symphony, et al.) effects */
        FX_LINE_JUMP => {
            /* !Tracker and Digital Symphony "Line Jump" */
            let ord = ctx.p.ord;
            libxmp_process_line_jump(&mut ctx.p.flow, ord, fxp as i32);
            None
        }
        FX_RETRIG => {
            /* Retrigger with extended range */
            Some(Label::Retrig)
        }

        _ => {
            libxmp_extras_process_fx(ctx, c, note, e.ins, fxt, fxp, fnum);
            None
        }
    };

    while let Some(label) = goto {
        goto = match label {
            Label::Arpeggio => {
                if !has_quirk!(QUIRK_ARPMEM) || fxp != 0 {
                    let xc = &mut xc!();
                    xc.arpeggio.val[0] = 0;
                    xc.arpeggio.val[1] = msn(fxp) as i8;
                    xc.arpeggio.val[2] = lsn(fxp) as i8;
                    xc.arpeggio.size = 3;
                }
                None
            }
            Label::Setpan => {
                let xc = &mut xc!();
                xc.pan.val = fxp as i32;
                xc.pan.surround = 0;
                xc.rpv = 0; /* storlek_20: set pan overrides random pan */
                None
            }
            Label::Volslide => 'b: {
                /* S3M file volume slide note:
                 * DFy  Fine volume down by y (...) If y is 0, the command will
                 *      be treated as a volume slide up with a value of f (15).
                 *      If a DFF command is specified, the volume will be slid
                 *      up.
                 */
                if has_quirk!(QUIRK_FINEFX) {
                    h = msn(fxp);
                    l = lsn(fxp);
                    if l == 0xf && h != 0 {
                        xc!().vol.memory = fxp as i32;
                        fxp >>= 4;
                        break 'b Some(Label::FVslideUp);
                    } else if h == 0xf && l != 0 {
                        xc!().vol.memory = fxp as i32;
                        fxp &= 0x0f;
                        break 'b Some(Label::FVslideDn);
                    }
                }

                /* recover memory */
                if fxp == 0x00 {
                    fxp = xc!().vol.memory as u8;
                    if fxp != 0 {
                        break 'b Some(Label::Volslide);
                    }
                }

                /* Skaven's 2nd reality (S3M) has volslide parameter D7 => pri
                 * down. Other trackers only compute volumes if the other
                 * parameter is 0, Fall from sky.xm has 2C => do nothing.
                 * Also don't assign xc->vol.memory if fxp is 0, see Guild
                 * of Sounds.xm
                 */
                if fxp != 0 {
                    let orpheus = ctx.m.flow_mode == FLOW_MODE_ORPHEUS;
                    let volpdn = has_quirk!(QUIRK_VOLPDN);
                    let xc = &mut xc!();
                    xc.set(VOL_SLIDE);
                    xc.vol.memory = fxp as i32;
                    h = msn(fxp);
                    l = lsn(fxp);
                    if fxp != 0 {
                        /* Imago Orpheus uses (x - y).
                         * TODO: a less hacky way of detecting this
                         * (READ_EVENT_ORPHEUS or quirk) would be nice.
                         * (test_effect_finefx_imf) */
                        if orpheus {
                            xc.vol.slide = h - l;
                        } else if volpdn {
                            xc.vol.slide = if l != 0 { -l } else { h };
                        } else {
                            xc.vol.slide = if h != 0 { h } else { -l };
                        }
                    }
                }

                /* Mirko reports that a S3M with D0F effects created with ST321
                 * should process volume slides in all frames like ST300. I
                 * suspect ST3/IT could be handling D0F effects like this.
                 */
                if has_quirk!(QUIRK_FINEFX) {
                    let xc = &mut xc!();
                    if msn(xc.vol.memory) == 0xf || lsn(xc.vol.memory) == 0xf {
                        xc.set(FINE_VOLS);
                        xc.vol.fslide = xc.vol.slide;
                    }
                }
                None
            }
            Label::Retrig => {
                let rtonce = has_quirk!(QUIRK_RTONCE);
                let xc = &mut xc!();
                xc.set(RETRIG);
                xc.retrig.val = fxp as i32;
                xc.retrig.count = fxp as i32 + 1;
                xc.retrig.type_ = 0;
                xc.retrig.limit = if rtonce { 1 } else { 0 };
                None
            }
            Label::FVslideUp => {
                let xc = &mut xc!();
                xc.set(FINE_VOLS);
                xc.vol.fslide = fxp as i32;
                None
            }
            Label::FVslideDn => {
                let xc = &mut xc!();
                xc.set(FINE_VOLS);
                xc.vol.fslide = -(fxp as i32);
                None
            }
            Label::FPortaUp => {
                if fxp != 0 {
                    let xc = &mut xc!();
                    xc.set(FINE_BEND);
                    xc.freq.fslide = -(fxp as f64);
                }
                None
            }
            Label::FPortaDn => {
                if fxp != 0 {
                    let xc = &mut xc!();
                    xc.set(FINE_BEND);
                    xc.freq.fslide = fxp as f64;
                }
                None
            }
            Label::XfPortaUp => {
                let xc = &mut xc!();
                xc.set(FINE_BEND);
                xc.freq.fslide = -0.25 * fxp as f64;
                None
            }
            Label::XfPortaDn => {
                let xc = &mut xc!();
                xc.set(FINE_BEND);
                xc.freq.fslide = 0.25 * fxp as f64;
                None
            }
            Label::PattDelay => {
                if ctx.m.read_event_type != READ_EVENT_ST3 || ctx.p.flow.delay == 0 {
                    ctx.p.flow.delay = fxp as i32;
                }
                None
            }
            Label::S3mSpeed => {
                if fxp != 0 {
                    ctx.p.speed = fxp as i32;
                    ctx.p.st26_speed = 0;
                }
                None
            }
            Label::S3mBpm => {
                /* Lower time factor in MED allows lower BPM values */
                let min_bpm = (0.5 + ctx.m.time_factor * XMP_MIN_BPM as f64 / 10.0) as i32;
                if (fxp as i32) < min_bpm {
                    fxp = min_bpm as u8;
                }
                ctx.p.bpm = fxp as i32;
                None
            }
            Label::Gvolslide => {
                if fxp != 0 {
                    let finefx = has_quirk!(QUIRK_FINEFX);
                    let xc = &mut xc!();
                    xc.set(GVOL_SLIDE);
                    xc.gvol.memory = fxp as i32;
                    h = msn(fxp);
                    l = lsn(fxp);

                    if finefx {
                        if l == 0xf && h != 0 {
                            xc.gvol.slide = 0;
                            xc.gvol.fslide = h;
                        } else if h == 0xf && l != 0 {
                            xc.gvol.slide = 0;
                            xc.gvol.fslide = -l;
                        } else {
                            xc.gvol.slide = if h != 0 { h } else { -l };
                            xc.gvol.fslide = 0;
                        }
                    } else {
                        xc.gvol.slide = if h != 0 { h } else { -l };
                        xc.gvol.fslide = 0;
                    }
                    None
                } else {
                    fxp = xc!().gvol.memory as u8;
                    if fxp != 0 {
                        Some(Label::Gvolslide)
                    } else {
                        None
                    }
                }
            }
            Label::TrkFvslide => {
                let xc = &mut xc!();
                xc.set(TRK_FVSLIDE);
                if fxp != 0 {
                    xc.trackvol.fslide = msn(fxp) - lsn(fxp);
                }
                None
            }
        };
    }
}
