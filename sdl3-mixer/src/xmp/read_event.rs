// Rust translation of src/read_event.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Reading the events of a row: notes, instruments and volumes, with the
//! quirks of each tracker family (Protracker, FT2, ST3, IT, MED).

use super::common::*;
use super::effects::*;
use super::lfo::*;
use super::loaders::med::*;
use super::med_extras::*;
use super::period::*;
use super::player::*;
use super::rng::libxmp_get_random;
use super::smix::libxmp_get_instrument;
use super::virt::*;
use super::*;

/// Translation of `get_subinstrument()`: the instrument and subinstrument
/// indices.
fn get_subinstrument(ctx: &ContextData, ins: i32, key: i32) -> Option<(usize, usize)> {
    let mod_ = &ctx.m.mod_;

    if is_valid_instrument(mod_, ins) {
        let instrument = &mod_.xxi[ins as usize];
        if is_valid_note(key) {
            let mapped = instrument.map[key as usize].ins as i32;
            if mapped != 0xff && mapped >= 0 && mapped < instrument.nsm {
                return Some((ins as usize, mapped as usize));
            }
        } else if mod_.xxi[ins as usize].nsm > 0 {
            return Some((ins as usize, 0));
        }
    }

    None
}

/// The subinstrument [`get_subinstrument()`] found.
fn sub_of(ctx: &ContextData, s: Option<(usize, usize)>) -> Option<XmpSubinstrument> {
    s.map(|(i, j)| ctx.m.mod_.xxi[i].sub[j])
}

/// Translation of `reset_envelopes()`.
fn reset_envelopes(ctx: &mut ContextData, chn: usize) {
    if !is_valid_instrument(&ctx.m.mod_, ctx.p.xc_data[chn].ins) {
        return;
    }
    let xc = &mut ctx.p.xc_data[chn];

    xc.reset_note(NOTE_ENV_END);

    xc.v_idx = -1;
    xc.p_idx = -1;
    xc.f_idx = -1;
}

/// Translation of `reset_envelope_volume()`.
fn reset_envelope_volume(ctx: &mut ContextData, chn: usize) {
    if !is_valid_instrument(&ctx.m.mod_, ctx.p.xc_data[chn].ins) {
        return;
    }
    let xc = &mut ctx.p.xc_data[chn];

    xc.reset_note(NOTE_ENV_END);

    xc.v_idx = -1;
}

/// Translation of `reset_envelopes_carry()`.
fn reset_envelopes_carry(ctx: &mut ContextData, chn: usize) {
    let ins = ctx.p.xc_data[chn].ins;
    if !is_valid_instrument(&ctx.m.mod_, ins) {
        return;
    }

    ctx.p.xc_data[chn].reset_note(NOTE_ENV_END);

    let xxi = libxmp_get_instrument(ctx, ins).unwrap();
    let (af, pf, ff) = (xxi.aei.flg, xxi.pei.flg, xxi.fei.flg);
    let xc = &mut ctx.p.xc_data[chn];

    /* Reset envelope positions */
    if !af & XMP_ENVELOPE_CARRY != 0 {
        xc.v_idx = -1;
    }
    if !pf & XMP_ENVELOPE_CARRY != 0 {
        xc.p_idx = -1;
    }
    if !ff & XMP_ENVELOPE_CARRY != 0 {
        xc.f_idx = -1;
    }
}

/// Translation of `set_effect_defaults()`.
fn set_effect_defaults(
    ctx: &mut ContextData,
    note: i32,
    sub: Option<XmpSubinstrument>,
    chn: usize,
    is_toneporta: i32,
) {
    let protrack = ctx.m.has_quirk(QUIRK_PROTRACK);
    let xc = &mut ctx.p.xc_data[chn];

    if let Some(sub) = sub {
        if note >= 0 {
            if !protrack {
                xc.finetune = sub.fin;
            }
            xc.gvl = sub.gvl;

            if sub.ifc & 0x80 != 0 {
                xc.filter.cutoff = (sub.ifc - 0x80) * 2;
            }
            xc.filter.envelope = 0x100;

            if sub.ifr & 0x80 != 0 {
                xc.filter.resonance = (sub.ifr - 0x80) * 2;
            }

            /* IT: on a new note without toneporta, allow a computed cutoff
             * of 127 with resonance 0 to disable the filter. */
            xc.filter.can_disable = (is_toneporta == 0) as i32;

            /* TODO: should probably expand the LFO period size instead
             * of reducing the vibrato rate precision here.
             */
            libxmp_lfo_set_depth(&mut xc.insvib.lfo, sub.vde);
            libxmp_lfo_set_rate(&mut xc.insvib.lfo, (sub.vra + 2) >> 2);
            libxmp_lfo_set_waveform(&mut xc.insvib.lfo, sub.vwf);
            xc.insvib.sweep = sub.vsw;

            libxmp_lfo_set_phase(&mut xc.vibrato.lfo, 0);
            libxmp_lfo_set_phase(&mut xc.tremolo.lfo, 0);
        }
    }

    xc.delay = 0;
    xc.tremor.up = 0;
    xc.tremor.down = 0;

    /* Reset arpeggio */
    xc.arpeggio.val[0] = 0;
    xc.arpeggio.count = 0;
    xc.arpeggio.size = 1;

    /* Reset toneporta--each libxmp_process_fx may add to the rate. */
    if is_toneporta != 0 {
        xc.porta.slide = 0;
    }
}

/// Translation of `set_channel_volume()`.
fn set_channel_volume(xc: &mut ChannelData, vol: i32) {
    if vol >= 0 {
        xc.volume = vol;
        xc.set(NEW_VOL);
    }
}

/// Translation of `set_channel_pan()`.
fn set_channel_pan(xc: &mut ChannelData, pan: i32) {
    /* TODO: LIQ supports surround for default panning, unclear if it works.
     * TODO: Imago Orpheus only temporarily sets channel panning for
     *       instrument numbers with no note. */
    if pan >= 0 {
        xc.pan.val = pan;
        xc.pan.surround = 0;
    }
}

/* From OpenMPT PortaTarget.mod:
 * "A new note (with no portamento command next to it) does not reset the
 *  portamento target. That is, if a previous portamento has not finished yet,
 *  calling 3xx or 5xx after the new note will slide it towards the old target.
 *  Once the portamento target period is reached, the target is reset. This
 *  means that if the period is modified by another slide (e.g. 1xx or 2xx),
 *  a following 3xx will not slide back to the original target."
 */
/// Translation of `set_period()`.
fn set_period(
    ctx: &mut ContextData,
    note: i32,
    sub: Option<XmpSubinstrument>,
    chn: usize,
    is_toneporta: i32,
) {
    let protrack = ctx.m.has_quirk(QUIRK_PROTRACK);

    /* TODO: blocking period updates on whether or not the event has a
     * valid instrument seems suspicious, but almost every format uses
     * this. Only allow Protracker to update without it for now. */
    if sub.is_none() && !protrack {
        return;
    }

    if note >= 0 {
        let (finetune, per_adj) = {
            let xc = &ctx.p.xc_data[chn];
            (xc.finetune, xc.per_adj)
        };
        let per = libxmp_note_to_period(ctx, note, finetune, per_adj);
        let xc = &mut ctx.p.xc_data[chn];

        if !protrack || (note > 0 && is_toneporta != 0) {
            xc.porta.target = per;
        }

        if xc.period < 1.0 || is_toneporta == 0 {
            xc.period = per;
        }
    }
}

/* From OpenMPT Porta-Pickup.xm:
 * "An instrument number should not reset the current portamento target. The
 *  portamento target is valid until a new target is specified by combining a
 *  note and a portamento effect."
 */
/// Translation of `set_period_ft2()`.
fn set_period_ft2(
    ctx: &mut ContextData,
    key: i32,
    note: i32,
    sub: Option<XmpSubinstrument>,
    chn: usize,
    is_toneporta: i32,
) {
    let (finetune, per_adj, ins) = {
        let xc = &ctx.p.xc_data[chn];
        (xc.finetune, xc.per_adj, xc.ins)
    };
    if is_valid_note(key - 1) && is_toneporta != 0 {
        /* Toneporta target updates even for invalid instruments, using
         * the default transpose +0 (ft2_invalid_porta_target.xm). */
        let mut n = key - 1;
        if let Some(sub) = sub {
            n += ctx.m.mod_.xxi[ins as usize].map[(key - 1) as usize].xpo as i32;
            n += sub.xpo;
        }
        let target = libxmp_note_to_period(ctx, n, finetune, per_adj);
        ctx.p.xc_data[chn].porta.target = target;
    }
    if sub.is_some() && note >= 0 {
        let xc = &ctx.p.xc_data[chn];
        if xc.period < 1.0 || is_toneporta == 0 {
            let period = libxmp_note_to_period(ctx, note, finetune, per_adj);
            ctx.p.xc_data[chn].period = period;
        }
    }
}

/// `IS_SFX_PITCH(x)`
#[inline]
fn is_sfx_pitch(x: u8) -> bool {
    x == FX_PITCH_ADD || x == FX_PITCH_SUB
}

/// `IS_TONEPORTA(x)`
#[inline]
fn is_toneporta_fx(x: u8) -> bool {
    x == FX_TONEPORTA
        || x == FX_TONE_VSLIDE
        || x == FX_PER_TPORTA
        || x == FX_ULT_TPORTA
        || x == FX_FAR_TPORTA
}

/// `IS_MOD_RETRIG(x, p)`
#[inline]
fn is_mod_retrig(x: u8, p: u8) -> bool {
    x == FX_EXTENDED && msn(p) == EX_RETRIG && lsn(p) != 0
}

/// `set_patch(ctx, chn, ins, smp, note)`
#[inline]
fn set_patch(ctx: &mut ContextData, chn: usize, ins: i32, smp: i32, note: i32) {
    libxmp_virt_setpatch(ctx, chn as i32, ins, smp, note, 0, 0, 0, 0);
}

/// Translation of `read_event_mod()`.
fn read_event_mod(ctx: &mut ContextData, e: &XmpEvent, chn: usize) -> i32 {
    let mut note: i32;
    let mut sub: Option<XmpSubinstrument> = None;
    let mut new_invalid_ins = 0;
    let mut new_swap_ins = 0;
    let mut is_toneporta: i32;
    let mut is_retrig: i32;
    let protrack = ctx.m.has_quirk(QUIRK_PROTRACK);

    ctx.p.xc_data[chn].flags = 0;
    note = -1;
    is_toneporta = 0;
    is_retrig = 0;

    if is_toneporta_fx(e.fxt) || is_toneporta_fx(e.f2t) {
        is_toneporta = 1;
    }
    if is_mod_retrig(e.fxt, e.fxp) || is_mod_retrig(e.f2t, e.f2p) {
        is_retrig = 1;
    }

    /* Check instrument */

    if is_valid_note(e.note as i32 - 1) && is_toneporta == 0 {
        ctx.p.xc_data[chn].key = e.note as i32 - 1;
    }
    if e.ins != 0 {
        let ins = e.ins as i32 - 1;
        {
            let xc = &mut ctx.p.xc_data[chn];
            xc.set(NEW_INS);
            xc.fadeout = 0x10000; /* for painlace.mod pat 0 ch 3 echo */
            /* TODO: FunkTracker: instruments probably don't reset
             * effects, investigate: fnk_note_vslide_cancel.fnk */
            xc.per_flags = 0;
            xc.offset.val = 0;
            xc.reset_note(NOTE_RELEASE | NOTE_FADEOUT);
        }

        if is_valid_instrument(&ctx.m.mod_, ins) {
            let key = ctx.p.xc_data[chn].key;
            sub = sub_of(ctx, get_subinstrument(ctx, ins, key));

            if let Some(s) = sub {
                new_swap_ins = 1;

                let xc = &mut ctx.p.xc_data[chn];
                /* Finetune is always loaded, but only applies
                 * when the period is updated by a note/porta
                 * (OpenMPT finetune.mod, PortaSwapPT.mod). */
                if protrack {
                    xc.finetune = s.fin;
                    xc.ins = ins;
                }

                /* Dennis Lindroos: instrument volume
                 * is not used on split channels
                 */
                if xc.split == 0 && e.note != XMP_KEY_OFF {
                    set_channel_volume(xc, s.vol);
                    set_channel_pan(xc, s.pan);
                }
            }

            if is_toneporta == 0 {
                let rls = ctx.m.mod_.xxi[ins as usize].rls;
                let xc = &mut ctx.p.xc_data[chn];
                xc.ins = ins;
                xc.ins_fade = rls;
            }
        } else {
            new_invalid_ins = 1;

            /* Invalid instruments do not reset the channel in
             * Protracker; instead, they set the current sample
             * to the invalid sample, which stops the current
             * sample at the end of its loop.
             *
             * OpenMPT PTInstrSwap.mod: uses a null sample to pause
             * a looping sample, plays several on a channel with no note.
             *
             * OpenMPT PTSwapEmpty.mod: repeatedly pauses and
             * restarts a sample using a null sample.
             */
            if !protrack || is_retrig != 0 {
                libxmp_virt_resetchannel(ctx, chn as i32);
            } else {
                libxmp_virt_queuepatch(ctx, chn as i32, -1, -1, 0);
            }
        }
    }

    /* Check note */

    if e.note != 0 {
        {
            let xc = &mut ctx.p.xc_data[chn];
            xc.set(NEW_NOTE);
            /* FunkTracker - new notes cancel persistent volume slide.
             * Farandole Composer notes are always paired with volume,
             * so this doesn't notably affect it. */
            xc.reset_per(VOL_SLIDE);
        }

        if e.note == XMP_KEY_OFF {
            ctx.p.xc_data[chn].set_note(NOTE_RELEASE);
        } else if is_toneporta == 0 && is_valid_note(e.note as i32 - 1) {
            ctx.p.xc_data[chn].reset_note(NOTE_END);

            let (xins, xkey) = (ctx.p.xc_data[chn].ins, ctx.p.xc_data[chn].key);
            sub = sub_of(ctx, get_subinstrument(ctx, xins, xkey));

            if let Some(s) = sub {
                let transp = ctx.m.mod_.xxi[xins as usize].map[xkey as usize].xpo as i32;

                note = xkey + s.xpo + transp;
                let mut smp = s.sid;

                if new_invalid_ins != 0 || !is_valid_sample(&ctx.m.mod_, smp) {
                    smp = -1;
                }

                if smp >= 0 && smp < ctx.m.mod_.smp {
                    set_patch(ctx, chn, xins, smp, note);
                    new_swap_ins = 0;
                    ctx.p.xc_data[chn].smp = smp;
                }
            } else {
                ctx.p.xc_data[chn].flags = 0;
                note = xkey;
            }
        }

        if note >= 0 {
            let xc = &mut ctx.p.xc_data[chn];
            xc.note = note;
            xc.set_note(NOTE_SET);
        }
    }

    /* Protracker 1/2 sample swap occurs when a sample number is
     * encountered without a note or with a note and toneporta. The new
     * instrument is switched to when the current sample reaches its loop
     * end. A valid note must have been played in this channel before.
     *
     * Empty samples can also be set, which stops the sample at the end
     * of its loop (see above).
     */
    if new_swap_ins != 0 && protrack && ctx.p.xc_data[chn].test_note(NOTE_SET) {
        if let Some(s) = sub {
            let smp = if is_valid_sample(&ctx.m.mod_, s.sid) {
                s.sid
            } else {
                -1
            };
            let xnote = ctx.p.xc_data[chn].note;
            libxmp_virt_queuepatch(ctx, chn as i32, e.ins as i32 - 1, smp, xnote);
            ctx.p.xc_data[chn].smp = s.sid;
        }
    }

    /* sub is now the currently playing subinstrument, which may not be
     * related to e->ins if there is active toneporta! */
    let (xins, xkey) = (ctx.p.xc_data[chn].ins, ctx.p.xc_data[chn].key);
    sub = sub_of(ctx, get_subinstrument(ctx, xins, xkey));

    set_effect_defaults(ctx, note, sub, chn, is_toneporta);
    if e.ins != 0 && sub.is_some() {
        reset_envelopes(ctx, chn);
    }

    /* Process new volume */
    set_channel_volume(&mut ctx.p.xc_data[chn], e.vol as i32 - 1);
    if e.vol != 0 {
        /* Farandole Composer - volume resets slide to volume. */
        ctx.p.xc_data[chn].reset_per(VOL_SLIDE);
    }

    /* Secondary effect handled first */
    libxmp_process_fx(ctx, chn as i32, e, 1);
    libxmp_process_fx(ctx, chn as i32, e, 0);

    if is_sfx_pitch(e.fxt) {
        let (finetune, per_adj) = {
            let xc = &ctx.p.xc_data[chn];
            (xc.finetune, xc.per_adj)
        };
        let period = libxmp_note_to_period(ctx, note, finetune, per_adj);
        ctx.p.xc_data[chn].period = period;
    } else {
        set_period(ctx, note, sub, chn, is_toneporta);
    }

    if sub.is_none() {
        return 0;
    }

    if note >= 0 && new_invalid_ins == 0 {
        let off = ctx.p.xc_data[chn].offset.val;
        libxmp_virt_voicepos(ctx, chn as i32, off as f64);
    } else if new_swap_ins != 0 && is_retrig != 0 && protrack {
        /* Protracker: an instrument number with no note and retrigger
         * triggers the new sample on tick 0. Other effects that set
         * RETRIG should not. (OpenMPT InstrSwapRetrigger.mod) */
        libxmp_virt_voicepos(ctx, chn as i32, 0.0);
    }

    let fx9bug = ctx.p.flags & XMP_FLAGS_FX9BUG != 0;
    let xc = &mut ctx.p.xc_data[chn];
    if xc.test(OFFSET) {
        if protrack || fx9bug {
            xc.offset.val = xc.offset.val.wrapping_add(xc.offset.val2);
        }
        xc.reset(OFFSET);
    }

    0
}

/// Translation of `sustain_check()`.
fn sustain_check(env: Option<&XmpEnvelope>, idx: i32) -> bool {
    match env {
        Some(env) => {
            (env.flg & XMP_ENVELOPE_ON != 0)
                && (env.flg & XMP_ENVELOPE_SUS != 0)
                && (!env.flg & XMP_ENVELOPE_LOOP != 0)
                && idx == env.d(env.sus << 1)
        }
        None => false,
    }
}

/// Translation of `read_event_ft2()`.
fn read_event_ft2(ctx: &mut ContextData, e: &XmpEvent, chn: usize) -> i32 {
    let mut note: i32;
    let mut key: i32;
    let sub: Option<XmpSubinstrument>;
    let mut is_toneporta: i32;
    let mut is_delayed: i32;
    let ft2bugs = ctx.m.has_quirk(QUIRK_FT2BUGS);

    /* From the OpenMPT DelayCombination.xm test case:
     * "Naturally, Fasttracker 2 ignores notes next to an out-of-range
     *  note delay. However, to check whether the delay is out of range,
     *  it is simply compared against the current song speed, not taking
     *  any pattern delays into account."
     */
    if ctx.p.frame >= ctx.p.speed {
        return 0;
    }

    let mut ev = *e;

    ctx.p.xc_data[chn].flags = 0;
    note = -1;
    key = ev.note as i32;
    is_toneporta = 0;
    is_delayed = 0;

    /* Delay has a few bizarre hacks that need to be supported. */
    if ev.fxt == FX_EXTENDED && msn(ev.fxp) == EX_DELAY && lsn(ev.fxp) != 0 {
        let xc = &ctx.p.xc_data[chn];
        /* No note + delay -> note memory (ft2_delay_note_memory.xm).
         * Combined with ins# memory, effectively causes a retrigger. */
        key = if key != 0 {
            key
        } else if xc.key_memory != 0 {
            xc.key_memory
        } else {
            xc.key + 1
        };

        /* Key off + no ins# + delay + volume column pan -> ignore pan
         * (OpenMPT PanOff.xm) (ft2_delay_volume_column.xm).
         */
        if ft2bugs && key == XMP_KEY_OFF as i32 && ev.ins == 0 && ev.f2t == FX_SETPAN {
            ev.f2t = 0;
            ev.f2p = 0;
        }
        is_delayed = 1;
    }

    if is_toneporta_fx(ev.fxt) || is_toneporta_fx(ev.f2t) {
        is_toneporta = 1;
        /* Mx + 3xx/5xy applies toneporta for both commands, but 3xx
         * uses the rate from the volume slot (ft2_double_toneporta.xm,
         * OpenMPT TonePortamentoMemory.xm). */
        if ft2bugs && ev.fxt == FX_TONEPORTA && is_toneporta_fx(ev.f2t) {
            ev.fxp = 0;
        }
    }

    /* FT2 deletes K00 and, if there is no volume fx toneporta, overwrites
     * the note with keyoff (ft2_k00_is_note_off.xm, OpenMPT key_off.xm,
     * ft2_envelope_reset.xm). */
    if ev.fxt == FX_KEYOFF && ev.fxp == 0 {
        ev.fxt = 0;
        if is_toneporta == 0 {
            key = XMP_KEY_OFF as i32;
        }
    }

    /* Check instrument
     *
     * Only update instrument/sample on new valid note + no toneporta/K00.
     * Lamb/forgotten city.xm relies heavily on quirks here.
     */
    if ev.ins != 0 {
        let xc = &mut ctx.p.xc_data[chn];
        xc.set(NEW_INS);
        xc.per_flags = 0; /* For posterity; not used by XM */
    }
    if is_valid_note(key - 1) && is_toneporta == 0 {
        let mut xxi_rls: Option<i32> = None;
        /* Note w/o instrument loads the last referenced instrument. */
        let ins = if ev.ins != 0 {
            ev.ins as i32
        } else {
            ctx.p.xc_data[chn].old_ins
        };
        let mut smp = -1;
        let mut n = key - 1;

        /* Updates on note + !toneporta + !K00 and is unaffected by
         * out-of-range transposition checks (ft2_note_range.xm). */
        ctx.p.xc_data[chn].key_memory = key;

        /* Unused instruments have fade 0x80, no envelopes, no vibrato.
         * Unused samples have volume 0, pan 0x80, transpose 0, no data.
         * libxmp represents unused/invalid instruments/samples as -1.
         * (test_player_ft2_note_noins_after_invalid_ins)
         * (test_player_ft2_note_off_after_invalid_ins)
         */
        if is_valid_instrument(&ctx.m.mod_, ins - 1) {
            let xxi = &ctx.m.mod_.xxi[(ins - 1) as usize];
            xxi_rls = Some(xxi.rls);
            let s = sub_of(ctx, get_subinstrument(ctx, ins - 1, key - 1));
            if let Some(s) = s {
                n += xxi.map[(key - 1) as usize].xpo as i32;
                n += s.xpo;
                smp = s.sid;
            }
        }
        /* TODO: out-of-range notes update envelopes, no ins.# req. */

        let valid_ins = is_valid_instrument(&ctx.m.mod_, ins - 1);
        let valid_smp = is_valid_sample(&ctx.m.mod_, smp);
        let xc = &mut ctx.p.xc_data[chn];

        /* Fade update requires ins.# (ft2_instrument_fade_update.xm).
         * Out-of-range notes update (ft2_note_range_instrument_fade.xm). */
        if ev.ins != 0 {
            xc.ins_fade = xxi_rls.unwrap_or(0x80 /* FT2 default */ << 1 /* conv */);
        }

        /* Valid notes update the instrument, sample, key, and note.
         * Note B-(-1) updates key/instrument/sample, but not the note.
         * Notes >A#9, <B-(-1) act as if the key does not exist at all.
         * (ft2_note_range.xm, OpenMPT NoteLimit.xm/NoteLimit2.xm)
         */
        if n >= FT2_NOTE_BN1 && n <= FT2_NOTE_AS9 {
            if n >= FT2_NOTE_C0 {
                xc.note = n;
            }
            xc.key = key - 1;
            xc.ins = if valid_ins { ins - 1 } else { -1 };
            xc.smp = if valid_smp { smp } else { -1 };
        } else {
            key = 0;
        }
    }
    /* Get the new instrument. If the instrument/key wasn't updated, this
     * is equivalent to FT2 retaining the previous instrument/sample. */
    let (xins, xkey) = (ctx.p.xc_data[chn].ins, ctx.p.xc_data[chn].key);
    sub = sub_of(ctx, get_subinstrument(ctx, xins, xkey));

    /* Check note
     *
     * Do not send a new note for toneporta (Quazar/funky stars.xm pos 5
     * ch 9, Mark Birch/comic bakery remix.xm pos 1 ch 3).
     */
    if key != 0 {
        ctx.p.xc_data[chn].set(NEW_NOTE);
    }
    if is_valid_note(key - 1) && is_toneporta == 0 {
        ctx.p.xc_data[chn].reset_note(NOTE_END);

        /* Send note even if the current sample is invalid.
         * Playing with an active invalid sample cuts the channel:
         * invalid instrument, valid instrument with invalid subins, or
         * zero-length sample (test_player_ft2_invalid_ins_defaults).
         */
        let (xins, xsmp, xnote) = {
            let xc = &ctx.p.xc_data[chn];
            (xc.ins, xc.smp, xc.note)
        };
        set_patch(ctx, chn, xins, xsmp, xnote);
        note = xnote;
    }

    /* Check key off/envelopes */

    if key == XMP_KEY_OFF as i32 {
        let xins = ctx.p.xc_data[chn].ins;
        let env = if is_valid_instrument(&ctx.m.mod_, xins) {
            Some(ctx.m.mod_.xxi[xins as usize].aei)
        } else {
            None
        };

        let xc = &mut ctx.p.xc_data[chn];
        if env.is_some() && (env.unwrap().flg & XMP_ENVELOPE_ON != 0) {
            if sustain_check(env.as_ref(), xc.v_idx) {
                /* See OpenMPT EnvOff.xm. In certain
                 * cases a release event is effective
                 * only in the next frame
                 */
                xc.set_note(NOTE_SUSEXIT);
            } else {
                xc.set_note(NOTE_RELEASE);
            }
        } else {
            /* No volume envelope -> cut volume to 0
             * (ft2_note_off_fade.xm, OpenMPT NoteOffVolume.xm). */
            set_channel_volume(xc, 0);
        }

        /* Keyoff always begins fadeout (ft2_note_off_fade.xm). */
        xc.set_note(NOTE_FADEOUT);
    }
    if (ev.ins != 0 && key != XMP_KEY_OFF as i32) || is_delayed != 0 {
        /* Reset release/fadeout for instrument numbers with no keyoff/K00
         * (xyce-dans_la_rue.xm chn 0 pat. 0E/0F, chn 10 pat. 0D/0E;
         * ft2_k00_defaults.xm; ft2_note_off_sustain.xm), and on
         * delayed rows (ft2_delay_envelope_*.xm, ft2_note_off_fade.xm,
         * OpenMPT NoteOff.xm). Other cases like note w/o ins# don't
         * reset fadeout (Cave Story - Last Battle.xm pos 11 chn 2,
         * ft2_note_no_fadeout_reset.xm).
         */
        {
            let xc = &mut ctx.p.xc_data[chn];
            xc.fadeout = 0x10000;
            xc.reset_note(NOTE_FADEOUT);
            xc.reset_note(NOTE_RELEASE | NOTE_SUSEXIT);
        }

        if sub.is_some() {
            /* Only reset envelopes with a valid active instrument
             * (ft2_envelope_invalid_ins.xm)
             * (Ghidorah/olympic dance.xm pos 10)
             * (Jeroen Tel/letting go.xm pos 4 chn 20).
             */
            reset_envelopes(ctx, chn);
        }

        /* Tremor count resets with fadeout (ft2_tremor_reset.xm). */
        ctx.p.xc_data[chn].tremor.count = TREMOR_SUPPRESS;
    }

    /* TODO: this function needs checking, probably split. */
    set_effect_defaults(ctx, note, sub, chn, is_toneporta);

    if ev.ins != 0 {
        /* Any ins.#: use active sample for defaults. Invalid samples
         * have volume 0 panning 0x80 (ft2_invalid_ins_defaults.xm).
         * Works on lines with K00 (ft2_k00_defaults.xm).
         */
        let xc = &mut ctx.p.xc_data[chn];
        set_channel_volume(xc, sub.map_or(0, |s| s.vol));
        set_channel_pan(xc, sub.map_or(0x80, |s| s.pan));
    }

    /* Process new volume */
    set_channel_volume(&mut ctx.p.xc_data[chn], ev.vol as i32 - 1);

    /* FT2: always reset sample offset */
    ctx.p.xc_data[chn].offset.val = 0;

    /* Secondary effect handled first */
    libxmp_process_fx(ctx, chn as i32, &ev, 1);
    libxmp_process_fx(ctx, chn as i32, &ev, 0);
    set_period_ft2(ctx, key, note, sub, chn, is_toneporta);
    /* TODO: Modplug, MT2, rstST process some FX every tick (affects toneporta memory). */

    if ctx.p.xc_data[chn].test(NEW_VOL) {
        /* Tremor is reset by ins# without keyoff or by delay rows.
         * Other events that set volume (volume column/Cxx, keyoff+ins#)
         * also temporarily override tremor, but don't reset it.
         * (Tremor likely just overwrites the channel volume in FT2.)
         * (ft2_tremor_reset.xm, OpenMPT TremorRecover.xm)
         */
        ctx.p.xc_data[chn].tremor.count |= TREMOR_SUPPRESS;
    }

    if note >= 0 {
        /* Sample offset requires valid note + 9xx + !toneporta
         * (Decibelter/Cosmic 'Wegian Mamas.xm pattern 4 channel 8).
         * In FT2, memory is set ONLY in these cases, and offsets past
         * the end of the sample cut. (Skale Tracker is different, so
         * limit this to QUIRK_FT2BUGS; reported by Vladislav Suschikh.)
         * (ft2_offset_memory.xm, OpenMPT 3xx-no-old-samp.xm/porta-offset.xm)
         */
        if ft2bugs && ctx.p.xc_data[chn].test(OFFSET) {
            let xc = &mut ctx.p.xc_data[chn];
            xc.offset.memory = (xc.offset.val & 0xff00) >> 8;

            let (xsmp, off) = (xc.smp, xc.offset.val);
            if !is_valid_sample(&ctx.m.mod_, xsmp) || off >= ctx.m.mod_.xxs[xsmp as usize].len {
                libxmp_virt_resetchannel(ctx, chn as i32);
            }
        }
        let off = ctx.p.xc_data[chn].offset.val;
        libxmp_virt_voicepos(ctx, chn as i32, off as f64);
    }

    0
}

/// Translation of `read_event_st3()`.
fn read_event_st3(ctx: &mut ContextData, e: &XmpEvent, chn: usize) -> i32 {
    let mut note: i32;
    let sub: Option<XmpSubinstrument>;
    let mut not_same_ins: i32;
    let mut is_toneporta: i32;

    ctx.p.xc_data[chn].flags = 0;
    note = -1;
    not_same_ins = 0;
    is_toneporta = 0;

    if is_toneporta_fx(e.fxt) || is_toneporta_fx(e.f2t) {
        is_toneporta = 1;
    }

    if libxmp_virt_mapchannel(ctx, chn as i32) < 0 && ctx.p.xc_data[chn].ins != e.ins as i32 - 1 {
        is_toneporta = 0;
    }

    /* Check instrument */

    if is_valid_note(e.note as i32 - 1) && is_toneporta == 0 {
        ctx.p.xc_data[chn].key = e.note as i32 - 1;
    }
    if e.ins != 0 {
        let ins = e.ins as i32 - 1;
        {
            let xc = &mut ctx.p.xc_data[chn];
            xc.set(NEW_INS);
            xc.fadeout = 0x10000;
            xc.per_flags = 0;
            xc.offset.val = 0;
            xc.reset_note(NOTE_RELEASE | NOTE_FADEOUT);
        }

        if is_valid_instrument(&ctx.m.mod_, ins) {
            if ctx.p.xc_data[chn].ins != ins {
                not_same_ins = 1;
                if is_toneporta == 0 {
                    let rls = ctx.m.mod_.xxi[ins as usize].rls;
                    let xc = &mut ctx.p.xc_data[chn];
                    xc.ins = ins;
                    xc.ins_fade = rls;
                }
            }

            /* Get new instrument volume */
            let key = ctx.p.xc_data[chn].key;
            let s = sub_of(ctx, get_subinstrument(ctx, ins, key));
            if let Some(s) = s {
                if e.note != XMP_KEY_OFF {
                    let xc = &mut ctx.p.xc_data[chn];
                    set_channel_volume(xc, s.vol);
                    set_channel_pan(xc, s.pan);
                }
            }
        } else {
            /* Ignore invalid instruments */
            ctx.p.xc_data[chn].flags = 0;
        }
    }

    /* Check note */

    if e.note != 0 {
        ctx.p.xc_data[chn].set(NEW_NOTE);

        if e.note == XMP_KEY_OFF {
            ctx.p.xc_data[chn].set_note(NOTE_RELEASE);
        } else if is_toneporta != 0 {
            /* Always retrig in tone portamento: Fix portamento in
             * 7spirits.s3m, mod.Biomechanoid
             */
            if not_same_ins != 0 {
                ctx.p.xc_data[chn].offset.val = 0;
            }
        } else if is_valid_note(e.note as i32 - 1) {
            ctx.p.xc_data[chn].reset_note(NOTE_END);

            let (xins, xkey) = (ctx.p.xc_data[chn].ins, ctx.p.xc_data[chn].key);
            let s = sub_of(ctx, get_subinstrument(ctx, xins, xkey));

            if let Some(s) = s {
                let transp = ctx.m.mod_.xxi[xins as usize].map[xkey as usize].xpo as i32;

                note = xkey + s.xpo + transp;
                let mut smp = s.sid;

                if !is_valid_sample(&ctx.m.mod_, smp) {
                    smp = -1;
                }

                if smp >= 0 && smp < ctx.m.mod_.smp {
                    set_patch(ctx, chn, xins, smp, note);
                    ctx.p.xc_data[chn].smp = smp;
                }
            } else {
                ctx.p.xc_data[chn].flags = 0;
            }
        }
    }

    /* sub is now the currently playing subinstrument, which may not be
     * related to e->ins if there is active toneporta! */
    let (xins, xkey) = (ctx.p.xc_data[chn].ins, ctx.p.xc_data[chn].key);
    sub = sub_of(ctx, get_subinstrument(ctx, xins, xkey));

    set_effect_defaults(ctx, note, sub, chn, is_toneporta);
    if e.ins != 0 && sub.is_some() {
        reset_envelopes(ctx, chn);
    }

    /* Process new volume */
    set_channel_volume(&mut ctx.p.xc_data[chn], e.vol as i32 - 1);

    /* Secondary effect handled first */
    libxmp_process_fx(ctx, chn as i32, e, 1);
    libxmp_process_fx(ctx, chn as i32, e, 0);
    set_period(ctx, note, sub, chn, is_toneporta);
    /* TODO: Orpheus processes some FX every tick (affects toneporta memory);
     * toneporta is additive only if both values are the same (otherwise,
     * use the value from slot 2). */

    if sub.is_none() {
        return 0;
    }

    if note >= 0 {
        ctx.p.xc_data[chn].note = note;
        let off = ctx.p.xc_data[chn].offset.val;
        libxmp_virt_voicepos(ctx, chn as i32, off as f64);
    }

    if ctx.m.has_quirk(QUIRK_ST3BUGS) && ctx.p.xc_data[chn].test(NEW_VOL) {
        let gvol = ctx.p.gvol;
        let volbase = ctx.m.volbase;
        let xc = &mut ctx.p.xc_data[chn];
        xc.volume = xc.volume * gvol / volbase;
    }

    0
}

/// Translation of `copy_channel()`.
#[inline]
fn copy_channel(p: &mut PlayerData, to: i32, from: i32) {
    if to > 0 && to != from {
        p.xc_data[to as usize] = p.xc_data[from as usize].clone();
    }
}

/// Translation of `check_fadeout()`.
fn check_fadeout(ctx: &ContextData, chn: usize, ins: i32) -> bool {
    let Some(xxi) = libxmp_get_instrument(ctx, ins) else {
        return true;
    };
    let xc = &ctx.p.xc_data[chn];

    (!xxi.aei.flg & XMP_ENVELOPE_ON != 0)
        || (!xxi.aei.flg & XMP_ENVELOPE_CARRY != 0)
        || xc.ins_fade == 0
        || xc.fadeout <= xc.ins_fade
}

/// Translation of `check_invalid_sample()`.
fn check_invalid_sample(ctx: &ContextData, ins: i32, key: i32) -> bool {
    let mod_ = &ctx.m.mod_;

    if ins < mod_.ins {
        let smp = mod_.xxi[ins as usize].map[key as usize].ins as i32;
        if smp == 0xff || smp >= mod_.smp {
            return true;
        }
    }

    false
}

/// Translation of `fix_period()`.
fn fix_period(ctx: &mut ContextData, chn: usize, sub: &XmpSubinstrument) {
    if sub.nna == XMP_INST_NNA_CONT {
        let xc = &ctx.p.xc_data[chn];
        let xxi = libxmp_get_instrument(ctx, xc.ins).unwrap();

        let n = xc.key + sub.xpo + xxi.map[xc.key_porta as usize].xpo as i32;
        let (finetune, per_adj) = (xc.finetune, xc.per_adj);
        let period = libxmp_note_to_period(ctx, n, finetune, per_adj);
        ctx.p.xc_data[chn].period = period;
    }
}

/// Translation of `is_same_sid()`.
fn is_same_sid(ctx: &ContextData, chn: usize, ins: i32, key: i32) -> bool {
    let xc = &ctx.p.xc_data[chn];

    let s1 = sub_of(ctx, get_subinstrument(ctx, ins, key));
    let s2 = sub_of(ctx, get_subinstrument(ctx, xc.ins, xc.key));

    matches!((s1, s2), (Some(s1), Some(s2)) if s1.sid == s2.sid)
}

/// Translation of `read_event_it()`.
fn read_event_it(ctx: &mut ContextData, e: &XmpEvent, chn: usize) -> i32 {
    let mut note: i32;
    let mut key: i32;
    let sub: Option<XmpSubinstrument>;
    let mut not_same_ins: i32;
    let mut not_same_smp: i32;
    let mut new_invalid_ins: i32;
    let mut is_toneporta: i32;
    let mut is_release: i32;
    let mut candidate_ins: i32;
    let mut reset_env: i32;
    let mut reset_susloop: i32;
    let mut use_ins_vol: i32;
    let sample_mode: i32;
    let mut toneporta_offset: i32;
    let mut retrig_ins: i32;

    let mut ev = *e;

    /* Emulate Impulse Tracker "always read instrument" bug */
    {
        let xc = &mut ctx.p.xc_data[chn];
        if ev.ins != 0 {
            xc.delayed_ins = 0;
        } else if ev.note != 0 && xc.delayed_ins != 0 {
            ev.ins = xc.delayed_ins as u8;
            xc.delayed_ins = 0;
        }
    }

    ctx.p.xc_data[chn].flags = 0;
    note = -1;
    key = ev.note as i32;
    not_same_ins = 0;
    not_same_smp = 0;
    new_invalid_ins = 0;
    is_toneporta = 0;
    is_release = 0;
    reset_env = 0;
    reset_susloop = 0;
    use_ins_vol = 0;
    candidate_ins = ctx.p.xc_data[chn].ins;
    sample_mode = !ctx.m.has_quirk(QUIRK_VIRTUAL) as i32;
    toneporta_offset = 0;
    retrig_ins = 0;

    /* Keyoff + instrument retrigs current instrument in old fx mode */
    if ctx.m.has_quirk(QUIRK_ITOLDFX)
        && ev.note == XMP_KEY_OFF
        && is_valid_instrument(&ctx.m.mod_, ev.ins as i32 - 1)
    {
        retrig_ins = 1;
    }

    /* Notes with unmapped instruments are ignored */
    if ev.ins != 0 {
        if ev.ins as i32 <= ctx.m.mod_.ins && is_valid_note(ev.note as i32 - 1) {
            let ins = ev.ins as i32 - 1;
            if check_invalid_sample(ctx, ins, ev.note as i32 - 1) {
                candidate_ins = ins;
                ev = XmpEvent::default();
            }
        }
    } else if is_valid_note(ev.note as i32 - 1) {
        let ins = ctx.p.xc_data[chn].old_ins - 1;
        if !is_valid_instrument(&ctx.m.mod_, ins) {
            new_invalid_ins = 1;
        } else if check_invalid_sample(ctx, ins, ev.note as i32 - 1) {
            ev = XmpEvent::default();
        }
    }

    if is_toneporta_fx(ev.fxt) || is_toneporta_fx(ev.f2t) {
        is_toneporta = 1;
    }

    if ctx.p.xc_data[chn].test_note(NOTE_ENV_RELEASE | NOTE_FADEOUT) {
        is_release = 1;
    }

    if ctx.p.xc_data[chn].period <= 0.0 || ctx.p.xc_data[chn].test_note(NOTE_END) {
        is_toneporta = 0;
    }

    /* Off-Porta.it */
    if is_toneporta != 0 && ev.fxt == FX_OFFSET {
        toneporta_offset = 1;
        if !ctx.m.has_quirk(QUIRK_PRENV) {
            ctx.p.xc_data[chn].reset_note(NOTE_ENV_END);
        }
    }

    /* Check instrument */

    if ev.ins != 0 {
        let ins = ev.ins as i32 - 1;
        let mut set_new_ins = 1;

        /* portamento_after_keyoff.it test case */
        if is_release != 0 && key == 0 {
            if is_toneporta != 0 {
                if ctx.m.has_quirk(QUIRK_PRENV) || ctx.p.xc_data[chn].test_note(NOTE_SET) {
                    is_toneporta = 0;
                    reset_envelopes_carry(ctx, chn);
                }
            } else {
                /* fixes OpenMPT wnoteoff.it */
                reset_envelopes_carry(ctx, chn);
            }
        }

        if is_toneporta != 0 && ctx.p.xc_data[chn].ins == ins && !ctx.m.has_quirk(QUIRK_PRENV) {
            if is_same_sid(ctx, chn, ins, key - 1) {
                /* same instrument and same sample */
                set_new_ins = (is_release == 0) as i32;
            } else {
                /* same instrument, different sample */
                not_same_ins = 1; /* need this too */
                not_same_smp = 1;
            }
        }

        if set_new_ins != 0 {
            ctx.p.xc_data[chn].set(NEW_INS);
            reset_env = 1;
        }
        /* Sample default volume is always enabled if a valid sample
         * is provided (Atomic Playboy, default_volume.it). */
        use_ins_vol = 1;
        ctx.p.xc_data[chn].per_flags = 0;

        if is_valid_instrument(&ctx.m.mod_, ins) {
            /* valid ins */

            /* See OpenMPT StoppedInstrSwap.it for cut case */
            if key == 0 && !ctx.p.xc_data[chn].test_note(NOTE_KEY_CUT) {
                /* Retrig in new ins in sample mode */
                if sample_mode != 0 && ctx.p.xc_data[chn].test_note(NOTE_END) {
                    libxmp_virt_voicepos(ctx, chn as i32, 0.0);
                }

                let xc = &mut ctx.p.xc_data[chn];
                /* IT: Reset note for every new != ins */
                if xc.ins == ins {
                    xc.set(NEW_INS);
                    use_ins_vol = 1;
                } else {
                    key = xc.key + 1;
                }

                xc.reset_note(NOTE_SET);
            }

            if ctx.p.xc_data[chn].ins != ins && (is_toneporta == 0 || !ctx.m.has_quirk(QUIRK_PRENV))
            {
                candidate_ins = ins;

                if !is_same_sid(ctx, chn, ins, key - 1) {
                    not_same_ins = 1;
                    if is_toneporta != 0 {
                        /* Get new instrument volume */
                        let s = sub_of(ctx, get_subinstrument(ctx, ins, key));
                        if let Some(s) = s {
                            ctx.p.xc_data[chn].volume = s.vol;
                            use_ins_vol = 0;
                        }
                    }
                }
            }
        } else {
            /* In sample mode invalid instruments cut the current
             * note (OpenMPT SampleNumberChange.it).
             * TODO: portamento_sustain.it order 3 row 19: when
             * sample release is set, this isn't always done? */
            if sample_mode != 0 {
                ctx.p.xc_data[chn].volume = 0;
            }

            /* Ignore invalid instruments */
            new_invalid_ins = 1;
            ctx.p.xc_data[chn].flags = 0;
            use_ins_vol = 0;
        }
    }

    /* Check note */

    if key != 0 {
        {
            let xc = &mut ctx.p.xc_data[chn];
            xc.set(NEW_NOTE);
            xc.set_note(NOTE_SET);
        }

        if key == XMP_KEY_FADE as i32 {
            ctx.p.xc_data[chn].set_note(NOTE_FADEOUT);
            reset_env = 0;
            reset_susloop = 0;
            use_ins_vol = 0;
        } else if key == XMP_KEY_CUT as i32 {
            let xc = &mut ctx.p.xc_data[chn];
            xc.set_note(NOTE_END | NOTE_CUT | NOTE_KEY_CUT);
            xc.period = 0.0;
            libxmp_virt_resetchannel(ctx, chn as i32);
        } else if key == XMP_KEY_OFF as i32 {
            let xins = ctx.p.xc_data[chn].ins;
            let env = if is_valid_instrument(&ctx.m.mod_, xins) {
                Some(ctx.m.mod_.xxi[xins as usize].aei)
            } else {
                None
            };
            let xc = &mut ctx.p.xc_data[chn];
            if sustain_check(env.as_ref(), xc.v_idx) {
                xc.set_note(NOTE_SUSEXIT);
            } else {
                xc.set_note(NOTE_RELEASE);
            }
            xc.set(KEY_OFF);
            /* Use instrument volume if an instrument was explicitly
             * provided on this row (see OpenMPT NoteOffInstr.it row 4).
             * However, never reset the envelope (see OpenMPT wnoteoff.it).
             */
            reset_env = 0;
            reset_susloop = 0;
            if ev.ins == 0 {
                use_ins_vol = 0;
            }
        } else if new_invalid_ins == 0 {
            /* Sample sustain release should always carry for tone
             * portamento, and is not reset unless a note is
             * present (Atomic Playboy, portamento_sustain.it). */
            /* portamento_after_keyoff.it test case */
            /* also see suburban_streets o13 c45 */
            if is_toneporta == 0 {
                reset_env = 1;
                reset_susloop = 1;
            }

            if is_toneporta != 0 {
                let xc = &mut ctx.p.xc_data[chn];
                if not_same_ins != 0 || xc.test_note(NOTE_END) {
                    xc.set(NEW_INS);
                    xc.reset_note(NOTE_ENV_RELEASE | NOTE_SUSEXIT | NOTE_FADEOUT);
                } else {
                    if is_valid_note(key - 1) {
                        xc.key_porta = key - 1;
                    }
                    key = 0;
                }
            }
        }
    }

    /* TODO: instrument change+porta(+release?) doesn't require a key.
     * Order 3/row 11 of portamento_sustain.it should change the sample. */
    if is_valid_note(key - 1) && new_invalid_ins == 0 {
        if ctx.p.xc_data[chn].test_note(NOTE_CUT) {
            use_ins_vol = 1; /* See OpenMPT NoteOffInstr.it */
        }
        key -= 1;
        ctx.p.xc_data[chn].key = key;
        ctx.p.xc_data[chn].reset_note(NOTE_END);

        let s = sub_of(ctx, get_subinstrument(ctx, candidate_ins, key));

        if let Some(s) = s {
            let transp = ctx.m.mod_.xxi[candidate_ins as usize].map[key as usize].xpo as i32;

            /* Clear note delay before duplicating channels:
             * it_note_delay_nna.it */
            ctx.p.xc_data[chn].delay = 0;

            note = key + s.xpo + transp;
            let mut smp = s.sid;
            if !is_valid_sample(&ctx.m.mod_, smp) {
                smp = -1;
            }
            let mut dct = s.dct;

            if not_same_smp != 0 {
                fix_period(ctx, chn, &s);
                /* Toneporta, even when not executed, disables
                 * NNA and DCAs for the current note:
                 * portamento_nna_sample.it, gxsmp2.it */
                libxmp_virt_setnna(ctx, chn as i32, XMP_INST_NNA_CUT);
                dct = XMP_INST_DCT_OFF;
            }
            let to = libxmp_virt_setpatch(
                ctx,
                chn as i32,
                candidate_ins,
                smp,
                note,
                key,
                s.nna,
                dct,
                s.dca,
            );

            /* Random value for volume swing */
            let mut rvv = s.rvv & 0xff;
            if rvv != 0 {
                clamp(&mut rvv, 0, 100);
                ctx.p.xc_data[chn].rvv = libxmp_get_random(&mut ctx.rng, rvv as u32 + 1) as i32;
            } else {
                ctx.p.xc_data[chn].rvv = 0;
            }

            /* Random value for pan swing */
            rvv = (s.rvv & 0xff00) >> 8;
            if rvv != 0 {
                clamp(&mut rvv, 0, 64);
                ctx.p.xc_data[chn].rpv =
                    libxmp_get_random(&mut ctx.rng, rvv as u32 + 1) as i32 - (rvv / 2);
            } else {
                ctx.p.xc_data[chn].rpv = 0;
            }

            if to < 0 {
                return -1;
            }
            if to != chn as i32 {
                copy_channel(&mut ctx.p, to, chn as i32);
                ctx.p.xc_data[to as usize].flags = 0;
            }

            if smp >= 0 {
                /* Not sure if needed */
                ctx.p.xc_data[chn].smp = smp;
            }
        } else {
            ctx.p.xc_data[chn].flags = 0;
            use_ins_vol = 0;
        }
    }

    /* Do after virtual channel copy */
    if (is_toneporta != 0 || retrig_ins != 0) && ctx.m.has_quirk(QUIRK_PRENV) && ev.ins != 0 {
        reset_envelopes_carry(ctx, chn);
    }

    if is_valid_instrument(&ctx.m.mod_, candidate_ins) {
        if ctx.p.xc_data[chn].ins != candidate_ins {
            /* Reset envelopes if instrument changes */
            reset_envelopes(ctx, chn);
        }
        let rls = ctx.m.mod_.xxi[candidate_ins as usize].rls;
        let xc = &mut ctx.p.xc_data[chn];
        xc.ins = candidate_ins;
        xc.ins_fade = rls;
    }

    /* Reset in case of new instrument and the previous envelope has
     * finished (OpenMPT test EnvReset.it). This must take place after
     * channel copies in case of NNA (see test/test.it)
     * Also if we have envelope in carry mode, check fadeout
     * Also, only reset the volume envelope. (it_fade_env_reset_carry.it)
     */
    if ev.ins != 0 && ctx.p.xc_data[chn].test_note(NOTE_ENV_END) {
        if check_fadeout(ctx, chn, candidate_ins) {
            reset_envelope_volume(ctx, chn);
        } else {
            reset_env = 0;
        }
    }

    if reset_env != 0 {
        let xc = &mut ctx.p.xc_data[chn];
        if ev.note != 0 {
            xc.reset_note(NOTE_ENV_RELEASE | NOTE_SUSEXIT | NOTE_FADEOUT);
        }
        /* Set after copying to new virtual channel (see ambio.it) */
        xc.fadeout = 0x10000;
    }
    if reset_susloop != 0 && ev.note != 0 {
        ctx.p.xc_data[chn].reset_note(NOTE_SAMPLE_RELEASE);
    }

    /* See OpenMPT wnoteoff.it vs noteoff3.it */
    if retrig_ins != 0 && not_same_ins != 0 {
        ctx.p.xc_data[chn].set(NEW_INS);
        libxmp_virt_voicepos(ctx, chn as i32, 0.0);
        let xc = &mut ctx.p.xc_data[chn];
        xc.fadeout = 0x10000;
        xc.reset_note(NOTE_RELEASE | NOTE_SUSEXIT | NOTE_FADEOUT);
    }

    let (xins, xkey) = (ctx.p.xc_data[chn].ins, ctx.p.xc_data[chn].key);
    sub = sub_of(ctx, get_subinstrument(ctx, xins, xkey));

    set_effect_defaults(ctx, note, sub, chn, is_toneporta);
    if let Some(s) = sub {
        if note >= 0 {
            /* Reset pan, see OpenMPT PanReset.it */
            set_channel_pan(&mut ctx.p.xc_data[chn], s.pan);

            if ctx.p.xc_data[chn].test_note(NOTE_CUT) {
                reset_envelopes(ctx, chn);
            } else if toneporta_offset == 0 || ctx.m.has_quirk(QUIRK_PRENV) {
                reset_envelopes_carry(ctx, chn);
            }
            ctx.p.xc_data[chn].reset_note(NOTE_CUT);
        }
    }

    /* Process new volume */
    if ev.vol != 0 && (!ctx.p.xc_data[chn].test_note(NOTE_CUT) || ev.ins != 0) {
        /* Do this even for XMP_KEY_OFF (see OpenMPT NoteOffInstr.it row 4). */
        set_channel_volume(&mut ctx.p.xc_data[chn], ev.vol as i32 - 1);
    }

    /* IT: always reset sample offset */
    ctx.p.xc_data[chn].offset.val &= !0xffff;

    /* According to Storlek test 25, Impulse Tracker handles the volume
     * column effects after the standard effects.
     * TODO: IT updates toneporta memory on first tick but reloads memory
     * to use for the rate for both channels every tick afterward.
     * TODO: Modplug processes some FX every tick (affects toneporta memory).
     */
    libxmp_process_fx(ctx, chn as i32, &ev, 0);
    libxmp_process_fx(ctx, chn as i32, &ev, 1);
    set_period(ctx, note, sub, chn, is_toneporta);

    let Some(s) = sub else {
        return 0;
    };

    if note >= 0 {
        ctx.p.xc_data[chn].note = note;
    }
    if note >= 0 || toneporta_offset != 0 {
        let mut off = 0;
        /* Offset >length starts at 0 (it_high_offset_memory.it) or at
         * sample end for old FX (it_high_offset_memory_oldfx.it). */
        let xc = &ctx.p.xc_data[chn];
        if xc.test(OFFSET)
            && (ctx.m.has_quirk(QUIRK_ITOLDFX)
                || (is_valid_sample(&ctx.m.mod_, xc.smp)
                    && xc.offset.val < ctx.m.mod_.xxs[xc.smp as usize].len))
        {
            off = xc.offset.val;
        }
        libxmp_virt_voicepos(ctx, chn as i32, off as f64);
    }

    let xc = &mut ctx.p.xc_data[chn];
    if use_ins_vol != 0 && !xc.test(NEW_VOL) {
        xc.volume = s.vol;
    }

    0
}

/// Translation of `read_event_med()`.
fn read_event_med(ctx: &mut ContextData, e: &XmpEvent, chn: usize) -> i32 {
    let mut note: i32;
    let sub: Option<XmpSubinstrument>;
    let mut new_invalid_ins = 0;
    let mut is_toneporta: i32;
    let tracker_version = med_module_extras(&ctx.m).unwrap().tracker_version;

    ctx.p.xc_data[chn].flags = 0;
    note = -1;
    is_toneporta = 0;

    /* TODO: 5xy (FX_TONE_VSLIDE) can't initiate toneporta,
     * only continue an active one. It does not set the target
     * even on an active toneporta? Instrument sets hold/decay
     * if 5xy is present. For all intents and purposes, 5xy
     * should not be setting this, it seems? */
    if e.fxt == FX_TONEPORTA || e.fxt == FX_TONE_VSLIDE {
        is_toneporta = 1;
    }

    /* Check instrument */

    if e.ins != 0 && e.note != 0 {
        let ins = e.ins as i32 - 1;
        {
            let xc = &mut ctx.p.xc_data[chn];
            xc.set(NEW_INS);
            xc.fadeout = 0x10000;
            xc.offset.val = 0;
            xc.reset_note(NOTE_RELEASE | NOTE_FADEOUT);
        }

        if is_valid_instrument(&ctx.m.mod_, ins) {
            if is_toneporta == 0 {
                let xxi = &ctx.m.mod_.xxi[ins as usize];
                let rls = xxi.rls;
                let hd = med_instrument_extras(xxi).map(|ie| (ie.hold, ie.decay));
                let xc = &mut ctx.p.xc_data[chn];
                xc.ins = ins;
                xc.ins_fade = rls;

                if let Some((hold, decay)) = hd {
                    libxmp_med_set_hold_decay(xc, hold, decay);
                }
            }
        } else {
            new_invalid_ins = 1;
            libxmp_virt_resetchannel(ctx, chn as i32);
        }

        let ce = med_channel_extras(&mut ctx.p.xc_data[chn]).unwrap();
        ce.arp = 0;
        ce.aidx = 0;
    }
    if e.ins != 0 {
        /* Hold symbols apply default volume from OctaMED 3.00 onward,
         * but they do not in older trackers. All events with a note
         * and an instrument apply default volume.
         */
        if is_valid_note(e.note as i32 - 1) || tracker_version >= MED_VER_OCTAMED_300 {
            /* Get new instrument volume */
            let s = sub_of(
                ctx,
                get_subinstrument(ctx, e.ins as i32 - 1, e.note as i32 - 1),
            );
            if let Some(s) = s {
                set_channel_volume(&mut ctx.p.xc_data[chn], s.vol);
            }
        }
    }

    /* Check note */

    if e.note != 0 {
        ctx.p.xc_data[chn].set(NEW_NOTE);

        let xins = ctx.p.xc_data[chn].ins;
        if e.note == XMP_KEY_OFF {
            ctx.p.xc_data[chn].set_note(NOTE_RELEASE);
        } else if e.note == XMP_KEY_CUT {
            let xc = &mut ctx.p.xc_data[chn];
            xc.set_note(NOTE_END);
            xc.period = 0.0;
            libxmp_virt_resetchannel(ctx, chn as i32);
        } else if is_toneporta == 0
            && is_valid_instrument(&ctx.m.mod_, xins)
            && is_valid_note(e.note as i32 - 1)
        {
            let xxi = &ctx.m.mod_.xxi[xins as usize];

            let mut per_adj = 0.0;
            if xxi.nsm > 1 {
                if let Some(ie) = med_instrument_extras(xxi) {
                    /* synth or iffoct */
                    if ie.vts == 0 && ie.wts == 0 {
                        /* iffoct */
                        per_adj = 2.0;
                    }
                }
            }
            {
                let xc = &mut ctx.p.xc_data[chn];
                xc.key = e.note as i32 - 1;
                xc.reset_note(NOTE_END);
                xc.per_adj = per_adj;
            }

            let xkey = ctx.p.xc_data[chn].key;
            let s = sub_of(ctx, get_subinstrument(ctx, xins, xkey));

            if let (0, Some(s)) = (new_invalid_ins, s) {
                let transp = ctx.m.mod_.xxi[xins as usize].map[xkey as usize].xpo as i32;

                note = xkey + s.xpo + transp;
                let mut smp = s.sid;

                if !is_valid_sample(&ctx.m.mod_, smp) {
                    smp = -1;
                }

                if smp >= 0 && smp < ctx.m.mod_.smp {
                    set_patch(ctx, chn, xins, smp, note);
                    ctx.p.xc_data[chn].smp = smp;
                }
            } else {
                ctx.p.xc_data[chn].flags = 0;
            }
        }
    }

    /* sub is now the currently playing subinstrument, which may not be
     * related to e->ins if there is active toneporta! */
    let (xins, xkey) = (ctx.p.xc_data[chn].ins, ctx.p.xc_data[chn].key);
    sub = sub_of(ctx, get_subinstrument(ctx, xins, xkey));

    /* Keep effect-set finetune if no instrument set */
    let finetune = ctx.p.xc_data[chn].finetune;
    set_effect_defaults(ctx, note, sub, chn, is_toneporta);
    if e.ins == 0 {
        ctx.p.xc_data[chn].finetune = finetune;
    }

    if e.ins != 0 && sub.is_some() {
        reset_envelopes(ctx, chn);
    }

    /* Process new volume */
    set_channel_volume(&mut ctx.p.xc_data[chn], e.vol as i32 - 1);

    /* Secondary effect handled first */
    libxmp_process_fx(ctx, chn as i32, e, 1);
    libxmp_process_fx(ctx, chn as i32, e, 0);
    set_period(ctx, note, sub, chn, is_toneporta);

    /* Test next line for a hold symbol. Hold is set either by the
     * instrument or by command 08 Hold/Decay, so test after effects. */
    libxmp_med_check_hold_symbol(ctx, chn);

    if sub.is_none() {
        return 0;
    }

    if note >= 0 {
        ctx.p.xc_data[chn].note = note;
        let off = ctx.p.xc_data[chn].offset.val;
        libxmp_virt_voicepos(ctx, chn as i32, off as f64);
    }

    0
}

/// Translation of `read_event_smix()`.
fn read_event_smix(ctx: &mut ContextData, e: &XmpEvent, chn: usize) -> i32 {
    ctx.p.xc_data[chn].flags = 0;

    if e.ins == 0 {
        return 0;
    }

    let ins = e.ins as i32 - 1;
    {
        let xc = &mut ctx.p.xc_data[chn];
        xc.set(NEW_INS);
        xc.per_flags = 0;
        xc.offset.val = 0;
        xc.reset_note(NOTE_RELEASE | NOTE_FADEOUT);
    }

    let rls = libxmp_get_instrument(ctx, ins).map(|x| x.rls);
    let xc = &mut ctx.p.xc_data[chn];
    if let Some(rls) = rls {
        xc.ins_fade = rls;
    }
    xc.ins = ins;

    xc.set(NEW_NOTE);

    if e.note == XMP_KEY_OFF {
        xc.set_note(NOTE_RELEASE);
        return 0;
    } else if e.note == XMP_KEY_FADE {
        xc.set_note(NOTE_FADEOUT);
        return 0;
    } else if e.note == XMP_KEY_CUT {
        xc.set_note(NOTE_END);
        xc.period = 0.0;
        libxmp_virt_resetchannel(ctx, chn as i32);
        return 0;
    }

    xc.key = e.note as i32 - 1;
    xc.fadeout = 0x10000;
    xc.reset_note(NOTE_END);

    let note;
    let sub;
    let mod_ins = ctx.m.mod_.ins;
    if ins >= mod_ins && ins < mod_ins + ctx.smix.ins {
        let xxi = &ctx.smix.xxi[(ins - mod_ins) as usize];
        if xxi.sub.is_empty() {
            return 0;
        }
        let s = xxi.sub[0];
        sub = Some(s);

        note = ctx.p.xc_data[chn].key + s.xpo;
        let mut smp = s.sid;
        if ctx.smix.xxs[smp as usize].len == 0 {
            smp = -1;
        }
        if smp >= 0 && smp < ctx.smix.smp {
            smp += ctx.m.mod_.smp;
            set_patch(ctx, chn, ins, smp, note);
            ctx.p.xc_data[chn].smp = smp;
        }
    } else {
        let xkey = ctx.p.xc_data[chn].key;
        let s = if is_valid_note(xkey) {
            sub_of(ctx, get_subinstrument(ctx, ins, xkey))
        } else {
            None
        };
        let Some(s) = s else {
            return 0;
        };
        sub = Some(s);
        let transp = libxmp_get_instrument(ctx, ins).unwrap().map[xkey as usize].xpo as i32;
        note = xkey + s.xpo + transp;
        let mut smp = s.sid;
        if !is_valid_sample(&ctx.m.mod_, smp) {
            smp = -1;
        }
        if smp >= 0 && smp < ctx.m.mod_.smp {
            set_patch(ctx, chn, ins, smp, note);
            ctx.p.xc_data[chn].smp = smp;
        }
    }

    set_effect_defaults(ctx, note, sub, chn, 0);
    set_period(ctx, note, sub, chn, 0);

    if e.ins != 0 {
        reset_envelopes(ctx, chn);
    }

    set_channel_volume(&mut ctx.p.xc_data[chn], e.vol as i32 - 1);

    ctx.p.xc_data[chn].note = note;
    let off = ctx.p.xc_data[chn].offset.val;
    libxmp_virt_voicepos(ctx, chn as i32, off as f64);

    0
}

/// Translation of `libxmp_read_event()`.
pub(crate) fn libxmp_read_event(ctx: &mut ContextData, e: &XmpEvent, chn: i32) -> i32 {
    let c = chn as usize;
    {
        let xc = &mut ctx.p.xc_data[c];

        if e.ins != 0 {
            xc.old_ins = e.ins as i32;
        }

        if xc.test_note(NOTE_SAMPLE_END) {
            xc.set_note(NOTE_END);
        }
    }

    if chn >= ctx.m.mod_.chn {
        return read_event_smix(ctx, e, c);
    }
    match ctx.m.read_event_type {
        READ_EVENT_MOD => read_event_mod(ctx, e, c),
        READ_EVENT_FT2 => read_event_ft2(ctx, e, c),
        READ_EVENT_ST3 => read_event_st3(ctx, e, c),
        READ_EVENT_IT => read_event_it(ctx, e, c),
        READ_EVENT_MED => read_event_med(ctx, e, c),
        _ => read_event_mod(ctx, e, c),
    }
}
