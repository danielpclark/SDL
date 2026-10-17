// Rust translation of src/far_extras.c and src/far_extras.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Farandole Composer tempo, vibrato, retrigger and slide effects.

use super::common::*;
use super::effects::*;
use super::extras::{ChannelExtras, ModuleExtras};
use super::lfo::*;
use super::period::*;
use super::player::*;
use super::*;

/// Translation of `struct far_channel_extras`.
#[derive(Clone, Debug, Default)]
pub(crate) struct FarChannelExtras {
    pub vib_sustain: i32, /* Is vibrato persistent? */
    pub vib_rate: i32,    /* Vibrato rate. */
}

/// Translation of `struct far_module_extras`.
#[derive(Clone, Debug, Default)]
pub(crate) struct FarModuleExtras {
    pub coarse_tempo: i32,
    pub fine_tempo: i32,
    pub tempo_mode: i32,
    pub vib_depth: i32, /* Vibrato depth for all channels. */
}

const FAR_GUS_CHANNELS: i32 = 17;
const FAR_OLD_TEMPO_MULT: i32 = 1 << 2; /* Number of real ticks for each FAR
                                         * "tick" in old tempo mode. In the
                                         * real replayer, this is 8. */

/*
 * The time factor needed to directly use FAR tempos is a little unintuitive.
 *
 * Generally: FAR tries to run 32/[coarse tempo] rows per second, which
 * (usually, but not always) are subdivided into 4 "ticks". To achieve
 * this, it measures tempos in the number of ticks that should play per second
 * (see far_tempos below). Fine tempo is added or subtracted from this number.
 * To time these ticks, FAR uses the programmable interval timer (PIT) to run a
 * player interrupt.
 *
 * libxmp effectively uses a calculation of 10.0 * 0.25 / BPM to get the tick
 * duration in seconds. A base time factor of 4.0 makes this 1 / BPM, turning
 * BPM into the ticks/sec measure that FAR uses. This isn't completely
 * accurate to FAR, though.
 *
 * The x86 PIT runs at a rate of 1193182 Hz, but FAR does something strange
 * when calculating PIT divisors and uses a constant of 1197255 Hz instead.
 * This means FAR tempo is slightly slower by a factor of around:
 *
 * floor(1197255 / 32) / floor(1193182 / 32) ~= 1.003439
 *
 * This still isn't perfect, but it gets the playback rate fairly close.
 */

/* tempo[0] = 256; tempo[i] = floor(128 / i). */
static FAR_TEMPOS: [i32; 16] = [
    256, 128, 64, 42, 32, 25, 21, 18, 16, 14, 12, 11, 10, 9, 9, 8,
];

/*
 * FAR tempo has some unusual requirements that don't really match any other
 * format:
 *
 * 1) The coarse tempo is roughly equivalent to speed, but a value of 0 is
 *    supported, and FAR doesn't actually have a concept of ticks: it translates
 *    this value to tempo.
 *
 * 2) There is some very bizarre clamping behavior involving fine tempo slides
 *    that needs to be emulated.
 *
 * 3) Tempos can range from 1 to 356(!). FAR uses a fixed row subdivision size
 *    of 16, so just shift the tempo by 4 and hope libxmp doesn't change it.
 *
 * 4) There are two tempo modes, and they can be switched between arbitrarily...
 */
/// Translation of `libxmp_far_translate_tempo()`.
pub(crate) fn libxmp_far_translate_tempo(
    mode: i32,
    fine_change: i32,
    coarse: i32,
    fine: &mut i32,
    speed_out: &mut i32,
    bpm_out: &mut i32,
) -> i32 {
    let mut speed;
    let mut bpm;

    if coarse < 0 || coarse > 15 || mode < 0 || mode > 1 {
        return -1;
    }

    /* Compatibility for FAR's broken fine tempo "clamping". */
    if fine_change < 0 && FAR_TEMPOS[coarse as usize] + *fine <= 0 {
        *fine = 0;
    } else if fine_change > 0 && FAR_TEMPOS[coarse as usize] + *fine >= 100 {
        *fine = 100;
    }

    if mode == 1 {
        /* "New" FAR tempo
         * Note that negative values are possible in Farandole Composer
         * via changing fine tempo and then slowing coarse tempo.
         * These result in very slow final tempos due to signed to
         * unsigned conversion. Zero should just be ignored entirely. */
        let tempo = FAR_TEMPOS[coarse as usize] + *fine;
        if tempo == 0 {
            return -1;
        }

        let mut divisor: u32 = (1197255 / tempo) as u32;

        /* Coincidentally(?), the "new" FAR tempo algorithm actually
         * prevents the BPM from dropping too far under XMP_MIN_BPM,
         * which is what libxmp needs anyway. */
        speed = 0;
        while divisor > 0xffff {
            divisor >>= 1;
            speed += 1;
        }
        if speed >= 2 {
            speed += 1;
        }
        speed += 3;
        /* Add an extra tick because the FAR replayer checks the tick
         * remaining count before decrementing it but after handling
         * each tick, i.e. a count of "3" executes 4 ticks. */
        speed += 1;
        /* TODO: non-integer BPMs: use PIT Hz here, make time factor 4.0? */
        bpm = (1197255u32 / divisor) as i32;
    } else {
        /* "Old" FAR tempo
         * This runs into the XMP_MIN_BPM limit, but nothing uses it anyway.
         * Old tempo mode in the original FAR replayer has 32 ticks,
         * but ignores all except every 8th. */
        /* TODO: this is also is affected by bad divisor math. */
        speed = 4 * FAR_OLD_TEMPO_MULT;
        bpm = (FAR_TEMPOS[coarse as usize] + *fine * 2) * FAR_OLD_TEMPO_MULT;
    }

    if bpm < XMP_MIN_BPM {
        bpm = XMP_MIN_BPM;
    }

    *speed_out = speed;
    *bpm_out = bpm;
    0
}

/// `FAR_MODULE_EXTRAS(m)`, if it has them.
pub(crate) fn far_module_extras(m: &mut ModuleData) -> Option<&mut FarModuleExtras> {
    match &mut m.extra {
        ModuleExtras::Far(e) => Some(e),
        _ => None,
    }
}

/// `FAR_MODULE_EXTRAS(m)`, which a module playing FAR effects has.
fn me_of(m: &mut ModuleData) -> &mut FarModuleExtras {
    far_module_extras(m).unwrap()
}

/// `FAR_CHANNEL_EXTRAS(xc)`, which a FAR module's channels have.
fn far_channel_extras(xc: &mut ChannelData) -> &mut FarChannelExtras {
    match &mut xc.extra {
        ChannelExtras::Far(e) => e,
        _ => unreachable!(),
    }
}

/// Translation of `libxmp_far_update_tempo()`.
fn libxmp_far_update_tempo(ctx: &mut ContextData, fine_change: i32) {
    let me = me_of(&mut ctx.m);
    let mut speed = 0;
    let mut bpm = 0;

    if libxmp_far_translate_tempo(
        me.tempo_mode,
        fine_change,
        me.coarse_tempo,
        &mut me.fine_tempo,
        &mut speed,
        &mut bpm,
    ) == 0
    {
        ctx.p.speed = speed;
        ctx.p.bpm = bpm;
    }
}

/// Translation of `libxmp_far_update_vibrato()`.
fn libxmp_far_update_vibrato(lfo: &mut Lfo, rate: i32, depth: i32) {
    libxmp_lfo_set_depth(
        lfo,
        libxmp_gus_frequency_steps(depth << 1, FAR_GUS_CHANNELS) as i32,
    );
    libxmp_lfo_set_rate(lfo, rate * 3);
}

/* Convoluted algorithm for delay times for retrigger and note offset effects. */
/// Translation of `libxmp_far_retrigger_delay()`.
fn libxmp_far_retrigger_delay(me: &FarModuleExtras, param: i32) -> i32 {
    if me.coarse_tempo < 0 || me.coarse_tempo > 15 || param < 1 {
        return -1;
    }

    let mut delay = (FAR_TEMPOS[me.coarse_tempo as usize] + me.fine_tempo) / param;

    if me.tempo_mode != 0 {
        /* Effects divide by 4, timer increments by 2 (round up). */
        ((delay >> 2) + 1) >> 1
    } else {
        /* Effects divide by 2, timer increments by 2 (round up).
         * Old tempo mode handles every 8th tick (* FAR_OLD_TEMPO_MULT).
         * Delay values >4 result in no retrigger. */
        delay = (((delay >> 1) + 1) >> 1) * FAR_OLD_TEMPO_MULT;
        if delay >= (4 * FAR_OLD_TEMPO_MULT) {
            return -1;
        }
        if delay < FAR_OLD_TEMPO_MULT {
            return FAR_OLD_TEMPO_MULT;
        }
        delay
    }
}

/// Translation of `libxmp_far_play_extras()`.
pub(crate) fn libxmp_far_play_extras(ctx: &mut ContextData, chn: usize) {
    let vib_depth = me_of(&mut ctx.m).vib_depth;
    let xc = &mut ctx.p.xc_data[chn];
    let vib_rate = far_channel_extras(xc).vib_rate;

    /* FAR vibrato depth is global, even though rate isn't. This might have
     * been changed by a different channel, so make sure it's applied. */
    if xc.test(VIBRATO) || xc.test_per(VIBRATO) {
        libxmp_far_update_vibrato(&mut xc.vibrato.lfo, vib_rate, vib_depth);
    }
}

/// Translation of `libxmp_far_new_channel_extras()`.
pub(crate) fn libxmp_far_new_channel_extras(xc: &mut ChannelData) -> i32 {
    xc.extra = ChannelExtras::Far(Box::default());
    0
}

/// Translation of `libxmp_far_reset_channel_extras()`.
pub(crate) fn libxmp_far_reset_channel_extras(xc: &mut ChannelData) {
    if let ChannelExtras::Far(ce) = &mut xc.extra {
        **ce = FarChannelExtras::default();
    }
}

/// Translation of `libxmp_far_new_module_extras()`.
pub(crate) fn libxmp_far_new_module_extras(m: &mut ModuleData) -> i32 {
    m.extra = ModuleExtras::Far(Box::new(FarModuleExtras {
        vib_depth: 4,
        ..Default::default()
    }));
    0
}

/// Translation of `libxmp_far_release_module_extras()`.
pub(crate) fn libxmp_far_release_module_extras(m: &mut ModuleData) {
    m.extra = ModuleExtras::None;
}

/// Translation of `libxmp_far_extras_process_fx()`.
pub(crate) fn libxmp_far_extras_process_fx(
    ctx: &mut ContextData,
    chn: usize,
    note: u8,
    _ins: u8,
    fxt: u8,
    mut fxp: u8,
    _fnum: i32,
) {
    let mut update_tempo = 0;
    let mut update_vibrato = 0;
    let mut fine_change = 0;

    /* Tempo effects and vibrato are multiplexed to reduce the effects count.
     *
     * Misc. notes: FAR pitch offset effects can overflow/underflow GUS
     * frequency, which isn't supported by libxmp (Haj/before.far).
     */
    match fxt {
        FX_FAR_PORTA_UP => {
            /* FAR pitch offset up */
            let xc = &mut ctx.p.xc_data[chn];
            xc.set(FINE_BEND);
            xc.reset_per(TONEPORTA);
            xc.freq.fslide = libxmp_gus_frequency_steps((fxp as i32) << 2, FAR_GUS_CHANNELS);
        }

        FX_FAR_PORTA_DN => {
            /* FAR pitch offset down */
            let xc = &mut ctx.p.xc_data[chn];
            xc.set(FINE_BEND);
            xc.reset_per(TONEPORTA);
            xc.freq.fslide = -libxmp_gus_frequency_steps((fxp as i32) << 2, FAR_GUS_CHANNELS);
        }

        /* Despite some claims, this effect scales with tempo and only
         * corresponds to (param) rows at tempo 4. See FORMATS.DOC.
         */
        FX_FAR_TPORTA => {
            /* FAR persistent tone portamento */
            if !is_valid_instrument(&ctx.m.mod_, ctx.p.xc_data[chn].ins) {
                return;
            }

            let me = me_of(&mut ctx.m);
            let mut tempo = FAR_TEMPOS[me.coarse_tempo as usize] + me.fine_tempo;

            ctx.p.xc_data[chn].set_per(TONEPORTA);
            if is_valid_note(note as i32 - 1) {
                let xc = &ctx.p.xc_data[chn];
                let (finetune, per_adj) = (xc.finetune, xc.per_adj);
                let target = libxmp_note_to_period(ctx, note as i32 - 1, finetune, per_adj);
                ctx.p.xc_data[chn].porta.target = target;
            }
            let xc = &mut ctx.p.xc_data[chn];
            xc.porta.dir = if xc.period < xc.porta.target { 1 } else { -1 };

            /* Parameter of 0 is equivalent to 1. */
            if fxp < 1 {
                fxp = 1;
            }
            /* Tempos <=0 cause crashes and other weird behavior
             * here in Farandole Composer, don't emulate that. */
            if tempo < 1 {
                tempo = 1;
            }

            let diff: i32 = (xc.porta.target - xc.period) as i32;
            let step: i32 = (if diff > 0 { diff } else { -diff }) * 8 / (tempo * fxp as i32);

            xc.porta.slide = if step > 0 { step } else { 1 };
        }

        /* Despite some claims, this effect scales with tempo and only
         * corresponds to (param/2) rows at tempo 4. See FORMATS.DOC.
         */
        FX_FAR_SLIDEVOL => {
            /* FAR persistent slide-to-volume */
            let me = me_of(&mut ctx.m);
            let mut tempo = FAR_TEMPOS[me.coarse_tempo as usize] + me.fine_tempo;
            let target = msn(fxp) << 4;
            fxp = lsn(fxp) as u8;

            /* Parameter of 0 is equivalent to 1. */
            if fxp < 1 {
                fxp = 1;
            }
            /* Tempos <=0 cause crashes and other weird behavior
             * here in Farandole Composer, don't emulate that. */
            if tempo < 1 {
                tempo = 1;
            }

            let xc = &mut ctx.p.xc_data[chn];
            let diff: i32 = target - xc.volume;
            let mut step: i32 = diff * 16 / (tempo * fxp as i32);
            if step == 0 {
                step = if diff > 0 { 1 } else { -1 };
            }

            xc.set_per(VOL_SLIDE);
            xc.vol.slide = step;
            xc.vol.target = target + 1;
        }

        FX_FAR_VIBDEPTH => {
            /* FAR set vibrato depth */
            me_of(&mut ctx.m).vib_depth = lsn(fxp);
            update_vibrato = 1;
        }

        FX_FAR_VIBRATO => {
            /* FAR vibrato and sustained vibrato */
            let xc = &mut ctx.p.xc_data[chn];
            let ce = far_channel_extras(xc);
            let mut set_vibrato = false;
            if ce.vib_sustain == 0 {
                /* With sustain, regular vibrato only sets the rate. */
                ce.vib_sustain = msn(fxp);
                if ce.vib_sustain == 0 {
                    set_vibrato = true;
                }
            }
            ce.vib_rate = lsn(fxp);
            if set_vibrato {
                xc.set(VIBRATO);
            }
            update_vibrato = 1;
        }

        /* Retrigger note param times at intervals that roughly evently
         * divide the row. A param of 0 crashes Farandole Composer.
         */
        FX_FAR_RETRIG => {
            /* FAR retrigger */
            let delay = libxmp_far_retrigger_delay(me_of(&mut ctx.m), fxp as i32);
            let speed = ctx.p.speed;
            if note != 0 && fxp > 1 && delay >= 0 && delay <= speed {
                let xc = &mut ctx.p.xc_data[chn];
                xc.set(RETRIG);
                xc.retrig.val = if delay != 0 { delay } else { 1 };
                xc.retrig.count = delay + 1;
                xc.retrig.type_ = 0;
                xc.retrig.limit = fxp as i32 - 1;
            }
        }

        /* A better effect name would probably be "retrigger once".
         * The description/intent seems to be that this is a delay
         * effect, but an initial note always plays as well. The second
         * note always plays on the (param)th tick due to player quirks,
         * but it's supposed to be derived similar to retrigger.
         * A param of zero works like effect 4F (bug?).
         */
        FX_FAR_DELAY => {
            /* FAR note offset */
            if note != 0 {
                let tempo_mode = me_of(&mut ctx.m).tempo_mode;
                let delay = if tempo_mode != 0 {
                    fxp as i32
                } else {
                    fxp as i32 * FAR_OLD_TEMPO_MULT
                };
                let xc = &mut ctx.p.xc_data[chn];
                xc.set(RETRIG);
                xc.retrig.val = if delay != 0 { delay } else { 1 };
                xc.retrig.count = delay + 1;
                xc.retrig.type_ = 0;
                xc.retrig.limit = if fxp != 0 { 1 } else { 0 };
            }
        }

        FX_FAR_TEMPO => {
            /* FAR coarse tempo and tempo mode */
            let me = me_of(&mut ctx.m);
            if msn(fxp) != 0 {
                me.tempo_mode = msn(fxp) - 1;
            } else {
                me.coarse_tempo = lsn(fxp);
            }
            update_tempo = 1;
        }

        FX_FAR_F_TEMPO => {
            /* FAR fine tempo slide up/down */
            let me = me_of(&mut ctx.m);
            if msn(fxp) != 0 {
                me.fine_tempo += msn(fxp);
                fine_change = msn(fxp);
            } else if lsn(fxp) != 0 {
                me.fine_tempo -= lsn(fxp);
                fine_change = -lsn(fxp);
            } else {
                me.fine_tempo = 0;
            }
            update_tempo = 1;
        }
        _ => {}
    }

    if update_vibrato != 0 {
        let vib_depth = me_of(&mut ctx.m).vib_depth;
        let xc = &mut ctx.p.xc_data[chn];
        let ce = far_channel_extras(xc);
        let mut set_per = None;
        if ce.vib_rate != 0 {
            if ce.vib_sustain != 0 {
                set_per = Some(true);
            }
        } else {
            set_per = Some(false);
            ce.vib_sustain = 0;
        }
        let vib_rate = ce.vib_rate;
        match set_per {
            Some(true) => xc.set_per(VIBRATO),
            Some(false) => xc.reset_per(VIBRATO),
            None => {}
        }
        libxmp_far_update_vibrato(&mut xc.vibrato.lfo, vib_rate, vib_depth);
    }

    if update_tempo != 0 {
        libxmp_far_update_tempo(ctx, fine_change);
    }
}
