// Rust translation of src/mixer.c and src/mixer.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The software mixer: voices, sample loops, volume ramps, anticlick and
//! the downmix to the output format.

use sdl3::stdlib::math::ceil;

use super::common::*;
use super::mix_all::*;
use super::mix_paula::{PaulaState, LIBXMP_A500LED_MIXERS, LIBXMP_A500_MIXERS};
use super::period::libxmp_note_to_period_mix;
use super::player::NOTE_SAMPLE_END;
use super::smix::libxmp_get_sample;
use super::virt::libxmp_virt_resetvoice;
use super::*;

pub(crate) const C4_PERIOD: f64 = 428.0;

pub(crate) const SMIX_NUMVOC: i32 = 128; /* default number of softmixer voices */
pub(crate) const SMIX_SHIFT: i32 = 16;
pub(crate) const SMIX_MASK: i32 = 0xffff;

pub(crate) const FILTER_SHIFT: i32 = 22;
pub(crate) const ANTICLICK_SHIFT: i32 = 3;

pub(crate) const PAN_SURROUND: i32 = 0x8000;

pub(crate) const VOICE_RELEASE: i32 = 1 << 0;
pub(crate) const ANTICLICK: i32 = 1 << 1;
pub(crate) const SAMPLE_LOOP: i32 = 1 << 2;
pub(crate) const VOICE_REVERSE: i32 = 1 << 3;
pub(crate) const VOICE_BIDIR: i32 = 1 << 4;
pub(crate) const SAMPLE_QUEUED: i32 = 1 << 5;
pub(crate) const SAMPLE_PAUSED: i32 = 1 << 6;

/// The `filter` of `struct mixer_voice`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VoiceFilter {
    pub r1: i32, /* filter variables */
    pub r2: i32,
    pub l1: i32,
    pub l2: i32,
    pub a0: i32,
    pub b0: i32,
    pub b1: i32,
    pub cutoff: i32,
    pub resonance: i32,
}

/// Translation of `struct mixer_voice`. `sptr` (the sample pointer) is
/// whether the voice's sample, `smp`, has data: upstream's pointer is
/// always that sample's data.
#[derive(Clone, Debug, Default)]
pub(crate) struct MixerVoice {
    pub chn: i32,                       /* channel number */
    pub root: i32,                      /* */
    pub note: i32,                      /* */
    pub pan: i32,                       /* */
    pub vol: i32,                       /* */
    pub period: f64,                    /* current period */
    pub pos: f64,                       /* position in sample */
    pub pos0: i32,                      /* position in sample before mixing */
    pub fidx: i32,                      /* mixer function index */
    pub ins: i32,                       /* instrument number */
    pub smp: i32,                       /* sample number */
    pub start: i32,                     /* loop start */
    pub end: i32,                       /* loop end */
    pub act: i32,                       /* nna info & status of voice */
    pub key: i32,                       /* key for DCA note check */
    pub old_vl: i32,                    /* previous volume, left channel */
    pub old_vr: i32,                    /* previous volume, right channel */
    pub sleft: i32,                     /* last left sample output, in 32bit */
    pub sright: i32,                    /* last right sample output, in 32bit */
    pub flags: i32,                     /* flags */
    pub sptr: bool,                     /* sample pointer */
    pub paula: Option<Box<PaulaState>>, /* paula simulation state */

    pub queued_smp: i32, /* Protracker queued instrument change */

    pub filter: VoiceFilter,
}

const DOWNMIX_SHIFT: i32 = 12;
const LIM8_HI: i32 = 127;
const LIM8_LO: i32 = -128;
const LIM16_HI: i32 = 32767;
const LIM16_LO: i32 = -32768;
const LIM32_HI: i32 = 2147483647;
const LIM32_LO: i32 = -2147483647 - 1;

const ANTICLICK_FPSHIFT: i32 = 24;

const LOOP_PROLOGUE: i32 = 1;
const LOOP_EPILOGUE: i32 = 2;

/// Translation of `struct loop_data` (`sptr` is the voice's sample).
#[derive(Default)]
struct LoopData {
    smp: i32,
    start: i32,
    end: i32,
    first_loop: i32,
    _16bit: i32,
    active: i32,
    prologue_num: i32,
    epilogue_num: i32,
    prologue: [u8; (LOOP_PROLOGUE * 2/* 16-bit */ * 2/* stereo */) as usize],
    epilogue: [u8; (LOOP_EPILOGUE * 2/* 16-bit */ * 2/* stereo */) as usize],
}

/* See mix_all.h for a full explanation of these flags. */
const FLAG_16_BITS: i32 = 0x01;
const FLAG_STEREO: i32 = 0x02;
const FLAG_STEREOOUT: i32 = 0x04;
const FLAG_FILTER: i32 = 0x08;
const FLAG_ACTIVE: i32 = 0x10;
/* #define FLAG_SYNTH	0x20 */
const FIDX_FLAGMASK: i32 = FLAG_16_BITS | FLAG_STEREO | FLAG_STEREOOUT | FLAG_FILTER;

/* Downmix 32bit samples to 8bit, signed or unsigned, mono or stereo output */
/// Translation of `downmix_int_8bit()`.
fn downmix_int_8bit(dest: &mut [u8], src: &[i32], num: usize, amp: i32, offs: u32) {
    let shift = DOWNMIX_SHIFT + 8 - amp;

    for i in 0..num {
        let smp = src[i] >> shift;
        let mut d: i8 = if smp > LIM8_HI {
            LIM8_HI as i8
        } else if smp < LIM8_LO {
            LIM8_LO as i8
        } else {
            smp as i8
        };

        if offs != 0 {
            d = (d as u8).wrapping_add(offs as u8) as i8;
        }
        dest[i] = d as u8;
    }
}

/* Downmix 32bit samples to 16bit, signed or unsigned, mono or stereo output */
/// Translation of `downmix_int_16bit()`.
fn downmix_int_16bit(dest: &mut [u8], src: &[i32], num: usize, amp: i32, offs: u32) {
    let shift = DOWNMIX_SHIFT - amp;

    for i in 0..num {
        let smp = src[i] >> shift;
        let mut d: i16 = if smp > LIM16_HI {
            LIM16_HI as i16
        } else if smp < LIM16_LO {
            LIM16_LO as i16
        } else {
            smp as i16
        };

        if offs != 0 {
            d = (d as u16).wrapping_add(offs as u16) as i16;
        }
        dest[i * 2..i * 2 + 2].copy_from_slice(&d.to_ne_bytes());
    }
}

/* Saturate 32bit samples, signed or unsigned, mono or stereo output */
/// Translation of `downmix_int_32bit()`.
fn downmix_int_32bit(dest: &mut [u8], src: &[i32], num: usize, amp: i32, offs: u32) {
    let shift = (16 - DOWNMIX_SHIFT) + amp;

    /* Signed left shift overflow is UB; clamp with pre-shifted bounds. */
    let max_value = LIM32_HI >> shift;
    let min_value = LIM32_LO >> shift;

    for i in 0..num {
        let smp = src[i];
        let mut d = if smp >= max_value {
            LIM32_HI
        } else if smp <= min_value {
            LIM32_LO
        } else {
            xmp_asl(smp, shift as u32)
        };

        if offs != 0 {
            d = (d as u32).wrapping_add(offs) as i32;
        }
        dest[i * 4..i * 4 + 4].copy_from_slice(&d.to_ne_bytes());
    }
}

/// Translation of `anticlick()`.
fn anticlick(vi: &mut MixerVoice) {
    vi.flags |= ANTICLICK;
    vi.old_vl = 0;
    vi.old_vr = 0;
}

/* Ok, it's messy, but it works :-) Hipolito */
/// Translation of `do_anticlick()`: `buf` is an index into `s->buf32`,
/// or `None` for its start.
fn do_anticlick(ctx: &mut ContextData, voc: usize, buf: Option<usize>, mut count: i32) {
    let s = &mut ctx.s;
    let vi = &mut ctx.p.virt.voice_array[voc];
    let discharge = s.ticksize >> ANTICLICK_SHIFT;

    let smp_l = vi.sleft;
    let smp_r = vi.sright;
    vi.sright = 0;
    vi.sleft = 0;

    if smp_l == 0 && smp_r == 0 {
        return;
    }

    let mut b = match buf {
        None => {
            count = discharge;
            0
        }
        Some(b) => {
            if count > discharge {
                count = discharge;
            }
            b
        }
    };

    if count <= 0 {
        return;
    }

    let stepval = (1 << ANTICLICK_FPSHIFT) / count;
    let mut stepmul = stepval * count;

    let buf32 = &mut s.buf32;
    if !s.format & XMP_FORMAT_MONO != 0 {
        loop {
            stepmul -= stepval;
            if stepmul <= 0 {
                break;
            }
            /* Truncate to 16-bits of precision so the product is 32-bits. */
            let mut stepmul_sq = (stepmul >> (ANTICLICK_FPSHIFT - 16)) as u32;
            stepmul_sq = stepmul_sq.wrapping_mul(stepmul_sq);
            buf32[b] = (buf32[b] as i64 + ((stepmul_sq as i64 * smp_l as i64) >> 32)) as i32;
            b += 1;
            buf32[b] = (buf32[b] as i64 + ((stepmul_sq as i64 * smp_r as i64) >> 32)) as i32;
            b += 1;
        }
    } else {
        loop {
            stepmul -= stepval;
            if stepmul <= 0 {
                break;
            }
            let mut stepmul_sq = (stepmul >> (ANTICLICK_FPSHIFT - 16)) as u32;
            stepmul_sq = stepmul_sq.wrapping_mul(stepmul_sq);
            buf32[b] = (buf32[b] as i64 + ((stepmul_sq as i64 * smp_l as i64) >> 32)) as i32;
            b += 1;
        }
    }
}

/// Translation of `set_sample_end()`.
fn set_sample_end(ctx: &mut ContextData, voc: i32, end: i32) {
    if voc as u32 >= ctx.p.virt.maxvoc as u32 {
        return;
    }

    let chn = ctx.p.virt.voice_array[voc as usize].chn;
    let xc = &mut ctx.p.xc_data[chn as usize];

    if end != 0 {
        xc.set_note(NOTE_SAMPLE_END);
        ctx.p.virt.voice_array[voc as usize].fidx &= !FLAG_ACTIVE;
        if ctx.m.has_quirk(QUIRK_RSTCHN) {
            libxmp_virt_resetvoice(ctx, voc, 0);
        }
    } else {
        xc.reset_note(NOTE_SAMPLE_END);
    }
}

/// The sample `smp` of the module, or of the sound effects
/// (`libxmp_get_sample()` without the bounds check).
fn xxs_of<'a>(m: &'a ModuleData, smix: &'a SmixData, smp: i32) -> &'a XmpSample {
    if smp < m.mod_.smp {
        &m.mod_.xxs[smp as usize]
    } else {
        &smix.xxs[(smp - m.mod_.smp) as usize]
    }
}

/// [`xxs_of()`], mutably.
fn xxs_of_mut<'a>(m: &'a mut ModuleData, smix: &'a mut SmixData, smp: i32) -> &'a mut XmpSample {
    if smp < m.mod_.smp {
        &mut m.mod_.xxs[smp as usize]
    } else {
        &mut smix.xxs[(smp - m.mod_.smp) as usize]
    }
}

/// Byte offset of sample `i` (of `size` bytes) in a sample's data.
#[inline]
fn soff(i: i32, size: i32) -> usize {
    (SAMPLE_GUARD as isize + (i as isize) * size as isize) as usize
}

/* Back up sample data before and after loop and replace it for interpolation.
 * TODO: if higher order interpolation than spline is added, the copy needs to
 *       properly wrap around the loop data (modulo) for correct small loops.
 * TODO: use an overlap buffer like OpenMPT? This is easier, but a little dirty. */
/// Translation of `init_sample_wraparound()`.
fn init_sample_wraparound(ctx: &mut ContextData, ld: &mut LoopData, voc: usize) {
    let mut prologue_num = LOOP_PROLOGUE;
    let mut epilogue_num = LOOP_EPILOGUE;

    let vi = &ctx.p.virt.voice_array[voc];
    let xxs = xxs_of_mut(&mut ctx.m, &mut ctx.smix, vi.smp);

    if !vi.sptr || ctx.s.interp == XMP_INTERP_NEAREST || (!xxs.flg & XMP_SAMPLE_LOOP) != 0 {
        ld.active = 0;
        return;
    }

    ld.smp = vi.smp;
    ld.start = vi.start;
    ld.end = vi.end;
    ld.first_loop = (vi.flags & SAMPLE_LOOP == 0) as i32;
    ld._16bit = xxs.flg & XMP_SAMPLE_16BIT;
    ld.active = 1;

    /* Stereo */
    if xxs.flg & XMP_SAMPLE_STEREO != 0 {
        ld.start <<= 1;
        ld.end <<= 1;
        prologue_num <<= 1;
        epilogue_num <<= 1;
    }
    ld.prologue_num = prologue_num;
    ld.epilogue_num = epilogue_num;

    let bidir = vi.flags & VOICE_BIDIR;
    let data = xxs.data.as_mut().unwrap();

    let size = if ld._16bit != 0 { 2 } else { 1 };
    let start = ld.start;
    let end = ld.end;

    let pn = (prologue_num * size) as usize;
    let en = (epilogue_num * size) as usize;
    let ps = soff(start - prologue_num, size);
    ld.prologue[..pn].copy_from_slice(&data[ps..ps + pn]);
    let es = soff(end, size);
    ld.epilogue[..en].copy_from_slice(&data[es..es + en]);

    let copy = |data: &mut Vec<u8>, to: i32, from: i32| {
        let t = soff(to, size);
        let f = soff(from, size);
        for k in 0..size as usize {
            data[t + k] = data[f + k];
        }
    };

    if ld.first_loop == 0 {
        for i in 0..prologue_num {
            let j = i - prologue_num;
            if bidir != 0 {
                copy(data, start + j, start - 1 - j);
            } else {
                copy(data, start + j, end + j);
            }
        }
    }
    for i in 0..epilogue_num {
        if bidir != 0 {
            copy(data, end + i, end - 1 - i);
        } else {
            copy(data, end + i, start + i);
        }
    }
}

/* Restore old sample data from before and after loop. */
/// Translation of `reset_sample_wraparound()`.
fn reset_sample_wraparound(ctx: &mut ContextData, ld: &LoopData) {
    let prologue_num = ld.prologue_num;
    let epilogue_num = ld.epilogue_num;

    if ld.active == 0 {
        return;
    }

    let xxs = xxs_of_mut(&mut ctx.m, &mut ctx.smix, ld.smp);
    let data = xxs.data.as_mut().unwrap();
    let size = if ld._16bit != 0 { 2 } else { 1 };

    let pn = (prologue_num * size) as usize;
    let en = (epilogue_num * size) as usize;
    let ps = soff(ld.start - prologue_num, size);
    data[ps..ps + pn].copy_from_slice(&ld.prologue[..pn]);
    let es = soff(ld.end, size);
    data[es..es + en].copy_from_slice(&ld.epilogue[..en]);
}

/// Translation of `has_active_sustain_loop()`.
fn has_active_sustain_loop(m: &ModuleData, vi: &MixerVoice, xxs: &XmpSample) -> bool {
    vi.smp < m.mod_.smp && (xxs.flg & XMP_SAMPLE_SLOOP != 0) && (!vi.flags & VOICE_RELEASE != 0)
}

/// Translation of `has_active_loop()`.
fn has_active_loop(m: &ModuleData, vi: &MixerVoice, xxs: &XmpSample) -> bool {
    (xxs.flg & XMP_SAMPLE_LOOP != 0) || has_active_sustain_loop(m, vi, xxs)
}

/* Update the voice endpoints based on current sample loop state. */
/// Translation of `adjust_voice_end()`.
fn adjust_voice_end(
    m: &ModuleData,
    vi: &mut MixerVoice,
    xxs: &XmpSample,
    xtra: Option<&ExtraSampleData>,
) {
    vi.flags &= !VOICE_BIDIR;

    let sus = match xtra {
        Some(xtra) if has_active_sustain_loop(m, vi, xxs) => Some(xtra),
        _ => None,
    };
    if let Some(xtra) = sus {
        vi.start = xtra.sus;
        vi.end = xtra.sue;
        if xxs.flg & XMP_SAMPLE_SLOOP_BIDIR != 0 {
            vi.flags |= VOICE_BIDIR;
        }
    } else if xxs.flg & XMP_SAMPLE_LOOP != 0 {
        vi.start = xxs.lps;
        if (xxs.flg & XMP_SAMPLE_LOOP_FULL != 0) && (!vi.flags & SAMPLE_LOOP != 0) {
            vi.end = xxs.len;
        } else {
            vi.end = xxs.lpe;
            if xxs.flg & XMP_SAMPLE_LOOP_BIDIR != 0 {
                vi.flags |= VOICE_BIDIR;
            }
        }
    } else {
        vi.start = 0;
        vi.end = xxs.len;
    }
}

/// The extra sample data of a voice's sample (`NULL` for sound effects).
fn xtra_of(m: &ModuleData, smp: i32) -> Option<&ExtraSampleData> {
    if smp < m.mod_.smp {
        Some(&m.xtra[smp as usize])
    } else {
        None
    }
}

/// Translation of `loop_reposition()`.
fn loop_reposition(ctx: &mut ContextData, voc: usize) -> i32 {
    let bidir_adjust = ctx.s.bidir_adjust;
    let vi = &mut ctx.p.virt.voice_array[voc];
    let m = &ctx.m;
    let xxs = xxs_of(m, &ctx.smix, vi.smp);
    let xtra = xtra_of(m, vi.smp);
    let loop_changed = (vi.flags & SAMPLE_LOOP == 0) as i32;

    vi.flags |= SAMPLE_LOOP;

    if loop_changed != 0 {
        adjust_voice_end(m, vi, xxs, xtra);
    }

    if !vi.flags & VOICE_BIDIR != 0 {
        /* Reposition for next loop */
        if !vi.flags & VOICE_REVERSE != 0 {
            vi.pos -= (vi.end - vi.start) as f64;
        } else {
            vi.pos += (vi.end - vi.start) as f64;
        }
    } else {
        /* Bidirectional loop: switch directions */
        vi.flags ^= VOICE_REVERSE;

        /* Wrap voice position around endpoint */
        if vi.flags & VOICE_REVERSE != 0 {
            /* OpenMPT Bidi-Loops.it: "In Impulse Tracker's software
             * mixer, ping-pong loops are shortened by one sample."
             */
            vi.pos = (vi.end * 2 - bidir_adjust) as f64 - vi.pos;
        } else {
            vi.pos = (vi.start * 2) as f64 - vi.pos;
        }
    }
    /* Safety check: pos should not be excessively past the sample end.
     * This only seems to happen with very low sample rates. */
    if vi.pos > (xxs.len + 1) as f64 {
        vi.pos = (xxs.len + 1) as f64;
    }
    loop_changed
}

/// Translation of `hotswap_sample()`.
fn hotswap_sample(ctx: &mut ContextData, voc: usize, smp: i32) {
    let vol = ctx.p.virt.voice_array[voc].vol;
    let pan = ctx.p.virt.voice_array[voc].pan;
    libxmp_mixer_setpatch(ctx, voc as i32, smp, 0);
    let vi = &mut ctx.p.virt.voice_array[voc];
    vi.flags |= SAMPLE_LOOP;
    vi.vol = vol;
    vi.pan = pan;
}

/// Translation of `get_current_sample()`: returns `c5spd` (the sample and
/// its extra data are the voice's `smp`).
fn get_current_sample(ctx: &mut ContextData, voc: usize) -> i32 {
    let m = &ctx.m;
    let mod_ = &m.mod_;
    let vi = &mut ctx.p.virt.voice_array[voc];

    let c5spd;
    if vi.smp < mod_.smp {
        c5spd = m.xtra[vi.smp as usize].c5spd as i32;
    } else {
        c5spd = m.c4rate;
    }
    let xxs = xxs_of(m, &ctx.smix, vi.smp);
    adjust_voice_end(m, vi, xxs, xtra_of(m, vi.smp));
    c5spd
}

/* Calculate the required number of sample frames to render a tick.
 * Returns -1 if any of the parameters are invalid. */
/// Translation of `libxmp_mixer_get_ticksize()`.
pub(crate) fn libxmp_mixer_get_ticksize(freq: i32, time_factor: f64, rrate: f64, bpm: i32) -> i32 {
    if freq <= 0 || bpm <= 0 || time_factor <= 0.0 || rrate <= 0.0 {
        return -1;
    }

    let calc = freq as f64 * time_factor * rrate / bpm as f64 / 1000.0;
    if calc > i32::MAX as f64 || calc.is_nan() {
        return -1;
    }

    let mut ticksize = calc as i32;

    if ticksize < (1 << ANTICLICK_SHIFT) {
        ticksize = 1 << ANTICLICK_SHIFT;
    }

    ticksize
}

/* Prepare the mixer for the next tick */
/// Translation of `libxmp_mixer_prepare()`.
fn libxmp_mixer_prepare(ctx: &mut ContextData) {
    let p = &ctx.p;
    let m = &ctx.m;
    let s = &mut ctx.s;

    s.ticksize = libxmp_mixer_get_ticksize(
        s.freq,
        m.time_factor * p.time_factor_relative,
        m.rrate,
        p.bpm,
    );

    /* Protect the mixer from broken values caused by xmp_set_tempo_factor. */
    if s.ticksize < 0 || s.ticksize > (s.total_size / 2) {
        s.ticksize = s.total_size / 2;
    }

    let mut len = s.ticksize as usize;
    if !s.format & XMP_FORMAT_MONO != 0 {
        len *= 2;
    }
    s.buf32[..len].fill(0);
}

/* Fill the output buffer calling one of the handlers. The buffer contains
 * sound for one tick (a PAL frame or 1/50s for standard vblank-timed mods)
 */
/// Translation of `libxmp_mixer_softmixer()`.
pub(crate) fn libxmp_mixer_softmixer(ctx: &mut ContextData) {
    let mut loop_data = LoopData::default();
    let mut samples: i32;
    let mut prev_l: i32;
    let mut prev_r: i32 = 0;

    let mut mixerset: &[Option<MixerFp>; 16] = match ctx.s.interp {
        XMP_INTERP_NEAREST => &LIBXMP_NEAREST_MIXERS,
        XMP_INTERP_LINEAR => &LIBXMP_LINEAR_MIXERS,
        XMP_INTERP_SPLINE => &LIBXMP_SPLINE_MIXERS,
        _ => &LIBXMP_LINEAR_MIXERS,
    };

    if ctx.p.flags & XMP_FLAGS_A500 != 0 && ctx.m.is_amiga_mod() {
        if ctx.p.filter != 0 {
            mixerset = &LIBXMP_A500LED_MIXERS;
        } else {
            mixerset = &LIBXMP_A500_MIXERS;
        }
    }

    /* OpenMPT Bidi-Loops.it: "In Impulse Tracker's software
     * mixer, ping-pong loops are shortened by one sample."
     */
    ctx.s.bidir_adjust = if ctx.m.is_player_mode_it() { 1 } else { 0 };

    libxmp_mixer_prepare(ctx);

    for voc in 0..ctx.p.virt.maxvoc as usize {
        let mut c5spd: i32;
        let mut rampsize: i32;
        let delta_l: i32;
        let delta_r: i32;
        let mut vol_l: i32;
        let mut vol_r: i32;

        if ctx.p.virt.voice_array[voc].flags & ANTICLICK != 0 {
            if ctx.s.interp > XMP_INTERP_NEAREST {
                do_anticlick(ctx, voc, None, 0);
            }
            ctx.p.virt.voice_array[voc].flags &= !ANTICLICK;
        }

        if ctx.p.virt.voice_array[voc].chn < 0 {
            continue;
        }

        if ctx.p.virt.voice_array[voc].period < 1.0 {
            libxmp_virt_resetvoice(ctx, voc as i32, 1);
            continue;
        }

        {
            let vi = &mut ctx.p.virt.voice_array[voc];
            /* Negative positions can be left over from some
             * loop edge cases. These can be safely clamped. */
            if vi.pos < 0.0 {
                vi.pos = 0.0;
            }

            vi.pos0 = vi.pos as i32;
        }

        let mut buf_pos: usize = 0;
        let mut vol = ctx.p.virt.voice_array[voc].vol;

        /* Mix volume (S3M and IT) */
        if ctx.m.mvolbase > 0 && ctx.m.mvol != ctx.m.mvolbase {
            vol = vol.wrapping_mul(ctx.m.mvol) / ctx.m.mvolbase;
        }

        {
            let vi = &ctx.p.virt.voice_array[voc];
            if vi.pan == PAN_SURROUND {
                vol_l = vol.wrapping_mul(0x80);
                vol_r = (-vol).wrapping_mul(0x80);
            } else {
                vol_l = vol.wrapping_mul(0x80 - vi.pan);
                vol_r = vol.wrapping_mul(0x80 + vi.pan);
            }
        }

        /* Sample is paused - skip channel unless a new sample is queued. */
        if ctx.p.virt.voice_array[voc].flags & SAMPLE_PAUSED != 0 {
            let vi = &mut ctx.p.virt.voice_array[voc];
            if (!vi.flags & SAMPLE_QUEUED != 0) || vi.queued_smp < 0 {
                vi.flags &= !SAMPLE_QUEUED;
                continue;
            }
            let q = vi.queued_smp;
            hotswap_sample(ctx, voc, q);
            c5spd = get_current_sample(ctx, voc);
            let vi = &mut ctx.p.virt.voice_array[voc];
            vi.pos = vi.start as f64;
        } else {
            c5spd = get_current_sample(ctx, voc);
        }

        let step =
            C4_PERIOD * c5spd as f64 / ctx.s.freq as f64 / ctx.p.virt.voice_array[voc].period;

        /* Don't allow <=0, otherwise m5v-nwlf.it crashes
         * Extremely high values that can cause undefined float/int
         * conversion are also possible for c5spd modules. */
        if step < 0.001 || step > i16::MAX as f64 {
            continue;
        }

        init_sample_wraparound(ctx, &mut loop_data, voc);

        rampsize = ctx.s.ticksize >> ANTICLICK_SHIFT;
        {
            let vi = &ctx.p.virt.voice_array[voc];
            delta_l = vol_l.wrapping_sub(vi.old_vl) / rampsize;
            delta_r = vol_r.wrapping_sub(vi.old_vr) / rampsize;
        }

        let mut size = ctx.s.ticksize;
        let mut usmp = ctx.s.ticksize;
        while size > 0 {
            let mut split_noloop = 0;
            let step_dir: f64;

            let vchn = ctx.p.virt.voice_array[voc].chn;
            if ctx.p.xc_data[vchn as usize].split != 0 {
                split_noloop = 1;
            }

            /* How many samples we can write before the loop break
             * or sample end... */
            {
                let vi = &ctx.p.virt.voice_array[voc];
                if !vi.flags & VOICE_REVERSE != 0 {
                    if vi.pos >= vi.end as f64 {
                        samples = 0;
                        usmp -= 1;
                        if usmp <= 0 {
                            break;
                        }
                    } else {
                        let mut c = ceil((vi.end as f64 - vi.pos) / step);
                        /* ...inside the tick boundaries */
                        if c > size as f64 {
                            c = size as f64;
                        }
                        samples = c as i32;
                    }
                    step_dir = step;
                } else {
                    /* Reverse */
                    if vi.pos <= vi.start as f64 {
                        samples = 0;
                        usmp -= 1;
                        if usmp <= 0 {
                            break;
                        }
                    } else {
                        let mut c = ceil((vi.pos - vi.start as f64) / step);
                        if c > size as f64 {
                            c = size as f64;
                        }
                        samples = c as i32;
                    }
                    step_dir = -step;
                }
            }

            if ctx.p.virt.voice_array[voc].vol != 0 {
                let mut mix_size = samples;
                let mut mixer_id = ctx.p.virt.voice_array[voc].fidx & FIDX_FLAGMASK;
                let stereo_out = !ctx.s.format & XMP_FORMAT_MONO != 0;

                if stereo_out {
                    mix_size *= 2;
                }

                /* For Hipolito's anticlick routine */
                if samples > 0 {
                    let b = &ctx.s.buf32;
                    if stereo_out {
                        prev_l = b[buf_pos + mix_size as usize - 2];
                        prev_r = b[buf_pos + mix_size as usize - 1];
                    } else {
                        prev_l = b[buf_pos + mix_size as usize - 1];
                    }
                } else {
                    prev_l = 0;
                    prev_r = 0;
                }

                {
                    let vi = &ctx.p.virt.voice_array[voc];
                    /* See OpenMPT env-flt-max.it */
                    if vi.filter.cutoff >= 0xfe && vi.filter.resonance == 0 {
                        mixer_id &= !FLAG_FILTER;
                    }
                }

                let mix_fn = mixerset[mixer_id as usize];

                /* Call the output handler */
                if samples > 0 && ctx.p.virt.voice_array[voc].sptr {
                    let mut rsize = 0;

                    if rampsize > samples {
                        rampsize -= samples;
                    } else {
                        rsize = samples - rampsize;
                        rampsize = 0;
                    }

                    if delta_l == 0 && delta_r == 0 {
                        /* no need to ramp */
                        rsize = samples;
                    }

                    if let Some(mix_fn) = mix_fn {
                        let vi = &mut ctx.p.virt.voice_array[voc];
                        let xxs = xxs_of_mut(&mut ctx.m, &mut ctx.smix, vi.smp);
                        mix_fn(
                            vi,
                            &mut ctx.s.buf32[buf_pos..],
                            xxs.data.as_mut().unwrap(),
                            samples,
                            vol_l >> 8,
                            vol_r >> 8,
                            (step_dir * (1 << SMIX_SHIFT) as f64) as i32,
                            rsize,
                            delta_l,
                            delta_r,
                        );
                    }

                    buf_pos += mix_size as usize;
                    let vi = &mut ctx.p.virt.voice_array[voc];
                    vi.old_vl = vi.old_vl.wrapping_add(samples.wrapping_mul(delta_l));
                    vi.old_vr = vi.old_vr.wrapping_add(samples.wrapping_mul(delta_r));

                    /* For Hipolito's anticlick routine */
                    let b = &ctx.s.buf32;
                    if stereo_out {
                        vi.sleft = b[buf_pos - 2].wrapping_sub(prev_l);
                        vi.sright = b[buf_pos - 1].wrapping_sub(prev_r);
                    } else {
                        vi.sleft = b[buf_pos - 1].wrapping_sub(prev_l);
                    }
                }
            }

            {
                let vi = &mut ctx.p.virt.voice_array[voc];
                vi.pos += step_dir * samples as f64;
            }
            size -= samples;

            /* One-shot samples do not loop. */
            let (active_loop, queued) = {
                let vi = &ctx.p.virt.voice_array[voc];
                let xxs = xxs_of(&ctx.m, &ctx.smix, vi.smp);
                (
                    has_active_loop(&ctx.m, vi, xxs),
                    vi.flags & SAMPLE_QUEUED != 0,
                )
            };
            if (!active_loop || split_noloop != 0) && !queued {
                if size > 0 {
                    do_anticlick(ctx, voc, Some(buf_pos), size);
                    set_sample_end(ctx, voc as i32, 1);
                    /* Next sample should ramp. */
                    vol_l = 0;
                    vol_r = 0;
                }
                size = 0;
                continue;
            }

            /* Loop before continuing to the next channel if the
             * tick is complete. This is particularly important
             * for reverse loops to avoid position clamping. */
            let past_end = {
                let vi = &ctx.p.virt.voice_array[voc];
                ((!vi.flags & VOICE_REVERSE != 0) && vi.pos >= vi.end as f64)
                    || ((vi.flags & VOICE_REVERSE != 0) && vi.pos <= vi.start as f64)
            };
            if size > 0 || past_end {
                if ctx.p.virt.voice_array[voc].flags & SAMPLE_QUEUED != 0 {
                    /* Protracker sample swap */
                    do_anticlick(ctx, voc, Some(buf_pos), size);
                    let (stop, q) = {
                        let vi = &ctx.p.virt.voice_array[voc];
                        let q = vi.queued_smp;
                        let stop = q < 0 || {
                            let xxs = xxs_of(&ctx.m, &ctx.smix, vi.smp);
                            !has_active_loop(&ctx.m, vi, xxs)
                                && (ctx.m.mod_.xxs[q as usize].flg & XMP_SAMPLE_LOOP == 0)
                        };
                        (stop, q)
                    };
                    if stop {
                        /* Invalid samples and one-shots that
                         * are being replaced by one-shots
                         * (OpenMPT PTStoppedSwap.mod) stop
                         * the current sample. If the current
                         * sample is looped, it needs to be paused.
                         */
                        let vi = &mut ctx.p.virt.voice_array[voc];
                        vi.flags &= !SAMPLE_QUEUED;
                        vi.flags |= SAMPLE_PAUSED;
                        set_sample_end(ctx, voc as i32, 1);
                        /* Next sample should ramp. */
                        vol_l = 0;
                        vol_r = 0;
                        size = 0;
                        continue;
                    }
                    reset_sample_wraparound(ctx, &loop_data);
                    hotswap_sample(ctx, voc, q);
                    c5spd = get_current_sample(ctx, voc);
                    let _ = c5spd;
                    init_sample_wraparound(ctx, &mut loop_data, voc);
                    let vi = &mut ctx.p.virt.voice_array[voc];
                    vi.pos = vi.start as f64;
                    continue;
                }
                if loop_reposition(ctx, voc) != 0 {
                    reset_sample_wraparound(ctx, &loop_data);
                    init_sample_wraparound(ctx, &mut loop_data, voc);
                }
            }
        }

        reset_sample_wraparound(ctx, &loop_data);
        let vi = &mut ctx.p.virt.voice_array[voc];
        vi.old_vl = vol_l;
        vi.old_vr = vol_r;
    }

    /* Render final frame */

    let s = &mut ctx.s;
    let mut size = s.ticksize;
    if !s.format & XMP_FORMAT_MONO != 0 {
        size *= 2;
    }

    if size > s.total_size {
        size = s.total_size;
    }

    let size = size as usize;
    if s.format & XMP_FORMAT_32BIT != 0 {
        let offs = if s.format & XMP_FORMAT_UNSIGNED != 0 {
            0x80000000u32
        } else {
            0
        };
        downmix_int_32bit(&mut s.buffer, &s.buf32, size, s.amplify, offs);
    } else if !s.format & XMP_FORMAT_8BIT != 0 {
        let offs = if s.format & XMP_FORMAT_UNSIGNED != 0 {
            0x8000u32
        } else {
            0
        };
        downmix_int_16bit(&mut s.buffer, &s.buf32, size, s.amplify, offs);
    } else {
        let offs = if s.format & XMP_FORMAT_UNSIGNED != 0 {
            0x80u32
        } else {
            0
        };
        downmix_int_8bit(&mut s.buffer, &s.buf32, size, s.amplify, offs);
    }

    s.dtright = 0;
    s.dtleft = 0;
}

/// Translation of `libxmp_mixer_voicepos()`.
pub(crate) fn libxmp_mixer_voicepos(ctx: &mut ContextData, voc: i32, pos: f64, ac: i32) {
    let voc_u = voc as usize;

    /* Position changes e.g. retrigger make the new sample take effect
     * if queued (OpenMPT InstrSwapRetrigger.mod). */
    if ctx.p.virt.voice_array[voc_u].flags & SAMPLE_QUEUED != 0 {
        let vi = &mut ctx.p.virt.voice_array[voc_u];
        vi.flags &= !SAMPLE_QUEUED;
        if vi.queued_smp < 0 {
            vi.flags |= SAMPLE_PAUSED;
        } else if vi.smp != vi.queued_smp {
            let q = vi.queued_smp;
            hotswap_sample(ctx, voc_u, q);
        }
        ctx.p.virt.voice_array[voc_u].flags |= SAMPLE_LOOP;
    }

    let m = &ctx.m;
    let vi = &mut ctx.p.virt.voice_array[voc_u];
    let xxs = xxs_of(m, &ctx.smix, vi.smp);
    let xtra = xtra_of(m, vi.smp);

    if xxs.flg & XMP_SAMPLE_SYNTH != 0 {
        return;
    }

    vi.pos = pos;

    adjust_voice_end(m, vi, xxs, xtra);

    let mut reposition = false;
    if vi.pos >= vi.end as f64 {
        vi.pos = vi.end as f64;
        /* Restart forward sample loops. */
        if (!vi.flags & VOICE_REVERSE != 0) && has_active_loop(m, vi, xxs) {
            reposition = true;
        }
    } else if (vi.flags & VOICE_REVERSE != 0) && vi.pos <= 0.1 {
        /* Hack: 0 maps to the end for reversed samples. */
        vi.pos = vi.end as f64;
    }
    if reposition {
        loop_reposition(ctx, voc_u);
    }

    if ac != 0 {
        anticlick(&mut ctx.p.virt.voice_array[voc_u]);
    }
}

/// Translation of `libxmp_mixer_getvoicepos()`.
pub(crate) fn libxmp_mixer_getvoicepos(ctx: &ContextData, voc: i32) -> f64 {
    let vi = &ctx.p.virt.voice_array[voc as usize];

    let xxs = libxmp_get_sample(ctx, vi.smp).unwrap();

    if xxs.flg & XMP_SAMPLE_SYNTH != 0 {
        return 0.0;
    }

    vi.pos
}

/// Translation of `libxmp_mixer_setpatch()`.
pub(crate) fn libxmp_mixer_setpatch(ctx: &mut ContextData, voc: i32, smp: i32, ac: i32) {
    let (has_data, flg) = {
        let xxs = libxmp_get_sample(ctx, smp).unwrap();
        (xxs.data.is_some(), xxs.flg)
    };
    let stereo_out = !ctx.s.format & XMP_FORMAT_MONO != 0;
    {
        let vi = &mut ctx.p.virt.voice_array[voc as usize];

        vi.smp = smp;
        vi.vol = 0;
        vi.pan = 0;
        vi.flags &= !(SAMPLE_LOOP | SAMPLE_QUEUED | SAMPLE_PAUSED | VOICE_REVERSE | VOICE_BIDIR);

        vi.fidx = 0;

        if stereo_out {
            vi.fidx |= FLAG_STEREOOUT;
        }
    }

    set_sample_end(ctx, voc, 0);

    /*mixer_setvol(ctx, voc, 0);*/

    let filter = ctx.m.has_quirk(QUIRK_FILTER) && ctx.s.dsp & XMP_DSP_LOWPASS != 0;
    let vi = &mut ctx.p.virt.voice_array[voc as usize];
    vi.sptr = has_data;
    vi.fidx |= FLAG_ACTIVE;

    if filter {
        vi.fidx |= FLAG_FILTER;
    }

    if flg & XMP_SAMPLE_16BIT != 0 {
        vi.fidx |= FLAG_16_BITS;
    }
    if flg & XMP_SAMPLE_STEREO != 0 {
        vi.fidx |= FLAG_STEREO;
    }

    libxmp_mixer_voicepos(ctx, voc, 0.0, ac);
}

/**
 * Replace the current playing sample when it reaches the end of its
 * sample loop, a la Protracker 1/2. The new sample will begin playing
 * at the start of its loop if it is looped, the start of the sample if
 * it is a one-shot, and it will not play and instead pause the channel
 * if both the original and the new sample are one-shots or if the new
 * sample is empty/invalid/-1.
 */
/// Translation of `libxmp_mixer_queuepatch()`.
pub(crate) fn libxmp_mixer_queuepatch(ctx: &mut ContextData, voc: i32, smp: i32) {
    let vi = &mut ctx.p.virt.voice_array[voc as usize];

    if smp != vi.smp || (vi.flags & SAMPLE_PAUSED != 0) {
        vi.queued_smp = smp;
        vi.flags |= SAMPLE_QUEUED;
    }
}

/// Translation of `libxmp_mixer_setnote()`.
pub(crate) fn libxmp_mixer_setnote(ctx: &mut ContextData, voc: i32, mut note: i32) {
    let vi = &mut ctx.p.virt.voice_array[voc as usize];

    /* FIXME: Workaround for crash on notes that are too high
     *        see 6nations.it (+114 transposition on instrument 16)
     */
    if note > 149 {
        note = 149;
    }

    vi.note = note;
    vi.period = libxmp_note_to_period_mix(note, 0);

    anticlick(vi);
}

/// Translation of `libxmp_mixer_setperiod()`.
pub(crate) fn libxmp_mixer_setperiod(ctx: &mut ContextData, voc: i32, period: f64) {
    let vi = &mut ctx.p.virt.voice_array[voc as usize];

    vi.period = period;
}

/// Translation of `libxmp_mixer_setvol()`.
pub(crate) fn libxmp_mixer_setvol(ctx: &mut ContextData, voc: i32, vol: i32) {
    let vi = &mut ctx.p.virt.voice_array[voc as usize];

    if vol == 0 {
        anticlick(vi);
    }

    vi.vol = vol;
}

/// Translation of `libxmp_mixer_release()`.
pub(crate) fn libxmp_mixer_release(ctx: &mut ContextData, voc: i32, rel: i32) {
    if rel != 0 {
        /* Cancel voice reverse when releasing an active sustain loop,
         * unless the main loop is bidirectional. This is done both for
         * bidirectional sustain loops and for forward sustain loops
         * that have been reversed with MPT S9F Play Backward. */
        let vi = &ctx.p.virt.voice_array[voc as usize];
        if !vi.flags & VOICE_RELEASE != 0 {
            let xxs = libxmp_get_sample(ctx, vi.smp).unwrap();

            if has_active_sustain_loop(&ctx.m, vi, xxs) && (!xxs.flg & XMP_SAMPLE_LOOP_BIDIR != 0) {
                ctx.p.virt.voice_array[voc as usize].flags &= !VOICE_REVERSE;
            }
        }
        ctx.p.virt.voice_array[voc as usize].flags |= VOICE_RELEASE;
    } else {
        ctx.p.virt.voice_array[voc as usize].flags &= !VOICE_RELEASE;
    }
}

/// Translation of `libxmp_mixer_reverse()`.
pub(crate) fn libxmp_mixer_reverse(ctx: &mut ContextData, voc: i32, rev: i32) {
    let vi = &mut ctx.p.virt.voice_array[voc as usize];

    /* Don't reverse samples that have already ended */
    if !vi.fidx & FLAG_ACTIVE != 0 {
        return;
    }

    if rev != 0 {
        vi.flags |= VOICE_REVERSE;
    } else {
        vi.flags &= !VOICE_REVERSE;
    }
}

/// Translation of `libxmp_mixer_seteffect()`.
pub(crate) fn libxmp_mixer_seteffect(ctx: &mut ContextData, voc: i32, type_: i32, val: i32) {
    let vi = &mut ctx.p.virt.voice_array[voc as usize];

    match type_ {
        DSP_EFFECT_CUTOFF => vi.filter.cutoff = val,
        DSP_EFFECT_RESONANCE => vi.filter.resonance = val,
        DSP_EFFECT_FILTER_A0 => vi.filter.a0 = val,
        DSP_EFFECT_FILTER_B0 => vi.filter.b0 = val,
        DSP_EFFECT_FILTER_B1 => vi.filter.b1 = val,
        _ => {}
    }
}

/// Translation of `libxmp_mixer_setpan()`.
pub(crate) fn libxmp_mixer_setpan(ctx: &mut ContextData, voc: i32, pan: i32) {
    let vi = &mut ctx.p.virt.voice_array[voc as usize];

    vi.pan = pan;
}

/// Translation of `libxmp_mixer_numvoices()`.
pub(crate) fn libxmp_mixer_numvoices(ctx: &ContextData, num: i32) -> i32 {
    let s = &ctx.s;

    if num > s.numvoc || num < 0 {
        s.numvoc
    } else {
        num
    }
}

/// Translation of `libxmp_mixer_on()`.
pub(crate) fn libxmp_mixer_on(ctx: &mut ContextData, rate: i32, format: i32, _c4rate: i32) -> i32 {
    let s = &mut ctx.s;
    let sample_size = if format & XMP_FORMAT_32BIT != 0 {
        4
    } else if !format & XMP_FORMAT_8BIT != 0 {
        2
    } else {
        1
    };

    let output_chn = if format & XMP_FORMAT_MONO != 0 { 1 } else { 2 };

    /* This should be equivalent to the XMP_MAX_FRAMESIZE calculation. */
    let mut total_size = output_chn
        * libxmp_mixer_get_ticksize(rate, DEFAULT_TIME_FACTOR * 2.0, PAL_RATE, XMP_MIN_BPM);
    if total_size < 0 {
        return -1;
    }

    /* These allocations were made with a fixed framesize based on the rate
     * 49170 for a long time, so make that the minimum size for now. */
    clamp(&mut total_size, 5 * 49170 * 2 / 20, XMP_MAX_FRAMESIZE);

    let Ok(buffer) = try_vec(total_size as usize * sample_size as usize, 0u8) else {
        return -1;
    };
    s.buffer = buffer;

    let Ok(buf32) = try_vec(total_size as usize, 0i32) else {
        s.buffer = Vec::new();
        return -1;
    };
    s.buf32 = buf32;

    s.total_size = total_size;
    s.sample_size = sample_size;
    s.output_chn = output_chn;
    s.freq = rate;
    s.format = format;
    s.amplify = DEFAULT_AMPLIFY;
    s.mix = DEFAULT_MIX;
    /* s->pbase = C4_PERIOD * c4rate / s->freq; */
    s.interp = XMP_INTERP_LINEAR; /* default interpolation type */
    s.dsp = XMP_DSP_LOWPASS; /* enable filters by default */
    /* s->numvoc = SMIX_NUMVOC; */
    s.dtright = 0;
    s.dtleft = 0;
    s.bidir_adjust = 0;

    0
}

/// Translation of `libxmp_mixer_off()`.
pub(crate) fn libxmp_mixer_off(ctx: &mut ContextData) {
    let s = &mut ctx.s;

    s.buffer = Vec::new();
    s.buf32 = Vec::new();
}
