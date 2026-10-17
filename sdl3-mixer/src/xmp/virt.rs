// Rust translation of src/virtual.c and src/virtual.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Virtual channels: the mapping of tracker channels (and the background
//! channels of IT new note actions) to mixer voices. (`virtual.c`; the
//! module is named `virt` as `virtual` is reserved in Rust.)

use super::common::*;
use super::mix_paula::{libxmp_paula_init, PaulaState};
use super::mixer::*;
use super::player::{libxmp_player_set_fadeout, libxmp_player_set_release};
use super::*;

pub(crate) const VIRT_ACTION_CUT: i32 = XMP_INST_NNA_CUT;
pub(crate) const VIRT_ACTION_CONT: i32 = XMP_INST_NNA_CONT;
pub(crate) const VIRT_ACTION_OFF: i32 = XMP_INST_NNA_OFF;
pub(crate) const VIRT_ACTION_FADE: i32 = XMP_INST_NNA_FADE;

pub(crate) const VIRT_ACTIVE: i32 = 0x100;
pub(crate) const VIRT_INVALID: i32 = -1;

const FREE: i32 = -1;

/* Get parent channel */
/// Translation of `libxmp_virt_getroot()`.
pub(crate) fn libxmp_virt_getroot(ctx: &ContextData, chn: i32) -> i32 {
    let p = &ctx.p;

    let voc = p.virt.virt_channel[chn as usize].map;
    if voc < 0 {
        return -1;
    }

    let vi = &p.virt.voice_array[voc as usize];

    vi.root
}

/// Translation of `do_virt_resetvoice()`.
fn do_virt_resetvoice(vi: &mut MixerVoice) {
    /* Preserve anticlick decay state through note cut. */
    let anticlick_l = vi.sleft;
    let anticlick_r = vi.sright;
    let flags = vi.flags & ANTICLICK;
    let paula = vi.paula.take();

    *vi = MixerVoice::default();

    vi.sleft = anticlick_l;
    vi.sright = anticlick_r;
    vi.flags = flags;
    vi.paula = paula;
    vi.chn = FREE;
    vi.root = FREE;
}

/// Translation of `libxmp_virt_resetvoice()`.
pub(crate) fn libxmp_virt_resetvoice(ctx: &mut ContextData, voc: i32, mute: i32) {
    if voc as u32 >= ctx.p.virt.maxvoc as u32 {
        return;
    }

    if mute != 0 {
        libxmp_mixer_setvol(ctx, voc, 0);
    }

    let p = &mut ctx.p;
    let (root, chn) = {
        let vi = &p.virt.voice_array[voc as usize];
        (vi.root, vi.chn)
    };
    p.virt.virt_used -= 1;
    p.virt.virt_channel[root as usize].count -= 1;
    p.virt.virt_channel[chn as usize].map = FREE;

    do_virt_resetvoice(&mut p.virt.voice_array[voc as usize]);
}

/* virt_on (number of tracks) */
/// Translation of `libxmp_virt_on()`.
pub(crate) fn libxmp_virt_on(ctx: &mut ContextData, num: i32) -> i32 {
    ctx.p.virt.num_tracks = num;
    let mut num = libxmp_mixer_numvoices(ctx, -1);

    ctx.p.virt.virt_channels = ctx.p.virt.num_tracks;

    if ctx.m.has_quirk(QUIRK_VIRTUAL) {
        ctx.p.virt.virt_channels += num;
    } else if num > ctx.p.virt.virt_channels {
        num = ctx.p.virt.virt_channels;
    }

    ctx.p.virt.maxvoc = libxmp_mixer_numvoices(ctx, num);

    let Ok(mut voice_array) = try_vec(ctx.p.virt.maxvoc.max(0) as usize, MixerVoice::default())
    else {
        return -1;
    };

    for vi in voice_array.iter_mut() {
        vi.chn = FREE;
        vi.root = FREE;
    }

    /* Initialize Paula simulator */
    if ctx.m.is_amiga_mod() {
        for vi in voice_array.iter_mut() {
            let mut paula = Box::new(PaulaState::default());
            libxmp_paula_init(ctx, &mut paula);
            vi.paula = Some(paula);
        }
    }
    ctx.p.virt.voice_array = voice_array;

    let Ok(virt_channel) = try_vec(
        ctx.p.virt.virt_channels.max(0) as usize,
        VirtChannel {
            map: FREE,
            count: 0,
        },
    ) else {
        ctx.p.virt.voice_array = Vec::new();
        return -1;
    };
    ctx.p.virt.virt_channel = virt_channel;

    ctx.p.virt.virt_used = 0;

    0
}

/// Translation of `libxmp_virt_off()`.
pub(crate) fn libxmp_virt_off(ctx: &mut ContextData) {
    let p = &mut ctx.p;

    /* Free Paula simulator state */
    /* Player type may have been changed; always free this. */

    p.virt.virt_used = 0;
    p.virt.maxvoc = 0;
    p.virt.virt_channels = 0;
    p.virt.num_tracks = 0;

    p.virt.voice_array = Vec::new();
    p.virt.virt_channel = Vec::new();
}

/// Translation of `libxmp_virt_reset()`.
pub(crate) fn libxmp_virt_reset(ctx: &mut ContextData) {
    let p = &mut ctx.p;

    if p.virt.virt_channels < 1 {
        return;
    }

    /* CID 129203 (#1 of 1): Useless call (USELESS_CALL)
     * Call is only useful for its return value, which is ignored.
     *
     * libxmp_mixer_numvoices(ctx, p->virt.maxvoc);
     */

    for i in 0..p.virt.maxvoc as usize {
        do_virt_resetvoice(&mut p.virt.voice_array[i]);
    }

    for i in 0..p.virt.virt_channels as usize {
        p.virt.virt_channel[i].map = FREE;
        p.virt.virt_channel[i].count = 0;
    }

    p.virt.virt_used = 0;
}

/// Translation of `free_voice()`.
fn free_voice(ctx: &mut ContextData) -> i32 {
    let p = &mut ctx.p;

    /* Find background voice with lowest volume*/
    let mut num = FREE;
    let mut vol = i32::MAX;
    for i in 0..p.virt.maxvoc as usize {
        let vi = &p.virt.voice_array[i];

        if vi.chn >= p.virt.num_tracks && vi.vol < vol {
            num = i as i32;
            vol = vi.vol;
        }
    }

    /* Free voice */
    if num >= 0 {
        let chn = p.virt.voice_array[num as usize].chn;
        let root = p.virt.voice_array[num as usize].root;
        p.virt.virt_channel[chn as usize].map = FREE;
        p.virt.virt_channel[root as usize].count -= 1;
        p.virt.virt_used -= 1;
    }

    num
}

/// Translation of `alloc_voice()`.
fn alloc_voice(ctx: &mut ContextData, chn: i32) -> i32 {
    let mut i = 0;

    /* Find free voice */
    while i < ctx.p.virt.maxvoc {
        if ctx.p.virt.voice_array[i as usize].chn == FREE {
            break;
        }
        i += 1;
    }

    /* not found */
    if i == ctx.p.virt.maxvoc {
        i = free_voice(ctx);
    }

    if i >= 0 {
        let p = &mut ctx.p;
        p.virt.virt_channel[chn as usize].count += 1;
        p.virt.virt_used += 1;

        p.virt.voice_array[i as usize].chn = chn;
        p.virt.voice_array[i as usize].root = chn;
        p.virt.virt_channel[chn as usize].map = i;
    }

    i
}

/// Translation of `map_virt_channel()`.
fn map_virt_channel(p: &PlayerData, chn: i32) -> i32 {
    if chn as u32 >= p.virt.virt_channels as u32 {
        return -1;
    }

    let voc = p.virt.virt_channel[chn as usize].map;

    if voc as u32 >= p.virt.maxvoc as u32 {
        return -1;
    }

    voc
}

/// Translation of `libxmp_virt_mapchannel()`.
pub(crate) fn libxmp_virt_mapchannel(ctx: &ContextData, chn: i32) -> i32 {
    map_virt_channel(&ctx.p, chn)
}

/// Translation of `libxmp_virt_resetchannel()`.
pub(crate) fn libxmp_virt_resetchannel(ctx: &mut ContextData, chn: i32) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    libxmp_virt_resetvoice(ctx, voc, 1);
}

/// Translation of `libxmp_virt_setvol()`.
pub(crate) fn libxmp_virt_setvol(ctx: &mut ContextData, chn: i32, mut vol: i32) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    let root = ctx.p.virt.voice_array[voc as usize].root;
    if root < XMP_MAX_CHANNELS as i32 && ctx.p.channel_mute[root as usize] != 0 {
        vol = 0;
    }

    libxmp_mixer_setvol(ctx, voc, vol);

    if vol == 0 && chn >= ctx.p.virt.num_tracks {
        libxmp_virt_resetvoice(ctx, voc, 1);
    }
}

/// Translation of `libxmp_virt_release()`.
pub(crate) fn libxmp_virt_release(ctx: &mut ContextData, chn: i32, rel: i32) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    libxmp_mixer_release(ctx, voc, rel);
}

/// Translation of `libxmp_virt_reverse()`.
pub(crate) fn libxmp_virt_reverse(ctx: &mut ContextData, chn: i32, rev: i32) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    libxmp_mixer_reverse(ctx, voc, rev);
}

/// Translation of `libxmp_virt_setpan()`.
pub(crate) fn libxmp_virt_setpan(ctx: &mut ContextData, chn: i32, pan: i32) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    libxmp_mixer_setpan(ctx, voc, pan);
}

/// Translation of `libxmp_virt_seteffect()`.
pub(crate) fn libxmp_virt_seteffect(ctx: &mut ContextData, chn: i32, type_: i32, val: i32) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    libxmp_mixer_seteffect(ctx, voc, type_, val);
}

/// Translation of `libxmp_virt_getvoicepos()`.
pub(crate) fn libxmp_virt_getvoicepos(ctx: &ContextData, chn: i32) -> f64 {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return -1.0;
    }

    libxmp_mixer_getvoicepos(ctx, voc)
}

/// Translation of `libxmp_virt_setsmp()`.
pub(crate) fn libxmp_virt_setsmp(ctx: &mut ContextData, chn: i32, smp: i32) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    if ctx.p.virt.voice_array[voc as usize].smp == smp {
        return;
    }

    let pos = libxmp_mixer_getvoicepos(ctx, voc);
    libxmp_mixer_setpatch(ctx, voc, smp, 0);
    libxmp_mixer_voicepos(ctx, voc, pos, 0); /* Restore old position */
}

/// Translation of `libxmp_virt_setnna()`.
pub(crate) fn libxmp_virt_setnna(ctx: &mut ContextData, chn: i32, nna: i32) {
    if !ctx.m.has_quirk(QUIRK_VIRTUAL) {
        return;
    }

    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    ctx.p.virt.voice_array[voc as usize].act = nna;
}

/// Translation of `check_dct()`.
fn check_dct(
    ctx: &mut ContextData,
    i: i32,
    chn: i32,
    ins: i32,
    smp: i32,
    key: i32,
    nna: i32,
    dct: i32,
    dca: i32,
) {
    let voc = ctx.p.virt.virt_channel[chn as usize].map;
    let vi = &mut ctx.p.virt.voice_array[i as usize];

    if vi.root == chn && vi.ins == ins {
        if nna == XMP_INST_NNA_CUT {
            libxmp_virt_resetvoice(ctx, i, 1);
            return;
        }

        vi.act = nna;

        if (dct == XMP_INST_DCT_INST)
            || (dct == XMP_INST_DCT_SMP && vi.smp == smp)
            || (dct == XMP_INST_DCT_NOTE && vi.key == key)
        {
            if nna == XMP_INST_NNA_OFF && dca == XMP_INST_DCA_FADE {
                vi.act = VIRT_ACTION_OFF;
            } else if dca != 0 {
                if i != voc || vi.act != 0 {
                    vi.act = dca;
                }
            } else {
                libxmp_virt_resetvoice(ctx, i, 1);
            }
        }
    }
}

/* For note slides */
/// Translation of `libxmp_virt_setnote()`.
pub(crate) fn libxmp_virt_setnote(ctx: &mut ContextData, chn: i32, note: i32) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    libxmp_mixer_setnote(ctx, voc, note);
}

/// Translation of `libxmp_virt_setpatch()`.
pub(crate) fn libxmp_virt_setpatch(
    ctx: &mut ContextData,
    mut chn: i32,
    ins: i32,
    mut smp: i32,
    note: i32,
    key: i32,
    nna: i32,
    dct: i32,
    dca: i32,
) -> i32 {
    if chn as u32 >= ctx.p.virt.virt_channels as u32 {
        return -1;
    }

    if ins < 0 {
        smp = -1;
    }

    if dct != 0 {
        for i in 0..ctx.p.virt.maxvoc {
            check_dct(ctx, i, chn, ins, smp, key, nna, dct, dca);
        }
    }

    let mut voc = ctx.p.virt.virt_channel[chn as usize].map;

    if voc > FREE {
        if ctx.p.virt.voice_array[voc as usize].act != 0 {
            let vfree = alloc_voice(ctx, chn);

            if vfree < 0 {
                return -1;
            }

            let p = &mut ctx.p;
            chn = p.virt.num_tracks;
            while chn < p.virt.virt_channels && {
                let used = p.virt.virt_channel[chn as usize].map > FREE;
                chn += 1;
                used
            } {}

            chn -= 1;
            p.virt.voice_array[voc as usize].chn = chn;
            p.virt.virt_channel[chn as usize].map = voc;
            voc = vfree;
        }
    } else {
        voc = alloc_voice(ctx, chn);
        if voc < 0 {
            return -1;
        }
    }

    if smp < 0 {
        libxmp_virt_resetvoice(ctx, voc, 1);
        return chn; /* was -1 */
    }

    libxmp_mixer_setpatch(ctx, voc, smp, 1);
    libxmp_mixer_setnote(ctx, voc, note);
    let vi = &mut ctx.p.virt.voice_array[voc as usize];
    vi.ins = ins;
    vi.act = nna;
    vi.key = key;

    chn
}

/// Translation of `libxmp_virt_queuepatch()`.
pub(crate) fn libxmp_virt_queuepatch(
    ctx: &mut ContextData,
    chn: i32,
    ins: i32,
    mut smp: i32,
    note: i32,
) -> i32 {
    /* Protracker 1/2 implements instrument swap in a strange way--the
     * volume/finetune take effect immediately but the sample change
     * does not apply until the current playing sample reaches the end of
     * its loop (or stops, if it's a one-off). */
    if chn as u32 >= ctx.p.virt.virt_channels as u32 {
        return -1;
    }

    if ins < 0 {
        smp = -1;
    }

    let voc = ctx.p.virt.virt_channel[chn as usize].map;
    if voc > FREE {
        libxmp_mixer_queuepatch(ctx, voc, smp);
        if ins >= 0 {
            ctx.p.virt.voice_array[voc as usize].ins = ins;
        }
        return chn;
    }
    /* Original sample stopped--start a new note. */
    if smp < 0 {
        return -1;
    }
    libxmp_virt_setpatch(ctx, chn, ins, smp, note, 0, 0, 0, 0)
}

/// Translation of `libxmp_virt_setperiod()`.
pub(crate) fn libxmp_virt_setperiod(ctx: &mut ContextData, chn: i32, period: f64) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    libxmp_mixer_setperiod(ctx, voc, period);
}

/// Translation of `libxmp_virt_voicepos()`.
pub(crate) fn libxmp_virt_voicepos(ctx: &mut ContextData, chn: i32, pos: f64) {
    let voc = map_virt_channel(&ctx.p, chn);
    if voc < 0 {
        return;
    }

    libxmp_mixer_voicepos(ctx, voc, pos, 1);
}

/// Translation of `libxmp_virt_pastnote()`.
pub(crate) fn libxmp_virt_pastnote(ctx: &mut ContextData, chn: i32, act: i32) {
    for c in ctx.p.virt.num_tracks..ctx.p.virt.virt_channels {
        let voc = map_virt_channel(&ctx.p, c);
        if voc < 0 {
            continue;
        }

        if ctx.p.virt.voice_array[voc as usize].root == chn {
            match act {
                VIRT_ACTION_CUT => libxmp_virt_resetvoice(ctx, voc, 1),
                VIRT_ACTION_OFF => libxmp_player_set_release(ctx, c),
                VIRT_ACTION_FADE => libxmp_player_set_fadeout(ctx, c),
                _ => {}
            }
        }
    }
}

/// Translation of `libxmp_virt_cstat()`.
pub(crate) fn libxmp_virt_cstat(ctx: &ContextData, chn: i32) -> i32 {
    let p = &ctx.p;

    let voc = map_virt_channel(p, chn);
    if voc < 0 {
        return VIRT_INVALID;
    }

    if chn < p.virt.num_tracks {
        return VIRT_ACTIVE;
    }

    p.virt.voice_array[voc as usize].act
}
