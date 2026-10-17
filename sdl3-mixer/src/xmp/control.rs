// Rust translation of src/control.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Context creation and the player controls: positions, seeking and the
//! player parameters.
//!
//! `xmp_next_position()`, `xmp_prev_position()`, `xmp_set_row()`,
//! `xmp_restart_module()`, `xmp_seek_time_frame()`, the channel mute and
//! volume controls, `xmp_get_format_list()`, `xmp_inject_event()`,
//! `xmp_set_instrument_path()` and the tempo factors aren't translated:
//! SDL_mixer doesn't call them.

use super::common::*;
use super::load::xmp_release_module;
use super::load_helpers::libxmp_set_player_mode;
use super::mixer::SMIX_NUMVOC;
use super::player::libxmp_reset_flow;
use super::rng::libxmp_init_random;
use super::scan::{libxmp_get_sequence, libxmp_scan_sequences};
use super::smix::xmp_end_smix;
use super::*;

/// Translation of `xmp_create_context()`.
pub(crate) fn xmp_create_context() -> XmpContext {
    let mut ctx = Box::<ContextData>::default();

    ctx.state = XMP_STATE_UNLOADED;
    ctx.m.defpan = 100;
    ctx.s.numvoc = SMIX_NUMVOC;
    libxmp_init_random(&mut ctx.rng);

    ctx
}

/// Translation of `xmp_free_context()`.
pub(crate) fn xmp_free_context(mut ctx: XmpContext) {
    xmp_free_context_data(&mut ctx);
}

/// What `xmp_free_context()` does before freeing the context (for an owner
/// that drops it later).
pub(crate) fn xmp_free_context_data(ctx: &mut ContextData) {
    if ctx.state > XMP_STATE_UNLOADED {
        xmp_release_module(ctx);
    }

    xmp_end_smix(ctx);

    ctx.m.instrument_path = None;
}

/// Translation of `set_position()`.
fn set_position(ctx: &mut ContextData, mut pos: i32, dir: i32) {
    /* If dir is 0, we can jump to a different sequence */
    let seq = if dir == 0 {
        libxmp_get_sequence(ctx, pos)
    } else {
        ctx.p.sequence
    };

    if seq < 0 || seq >= ctx.m.num_sequences {
        return;
    }

    let has_marker = ctx.m.has_quirk(QUIRK_MARKER);

    ctx.p.sequence = seq;

    if pos >= 0 {
        let mod_ = &ctx.m.mod_;
        let p = &mut ctx.p;
        let f = &mut p.flow;

        while has_marker && pos > 0 && pos < mod_.len - 1 && mod_.xxo[pos as usize] == XMP_MARK_SKIP
        {
            if dir < 0 {
                pos -= 1;
            } else {
                pos += 1;
            }
        }
        if pos >= mod_.len {
            return;
        }
        let pat = mod_.xxo[pos as usize] as i32;

        if pat < mod_.pat {
            if has_marker && pat == XMP_MARK_END as i32 {
                return;
            }

            if pos > p.scan[seq as usize].ord {
                f.end_point = 0;
            } else {
                f.num_rows = mod_.pattern(pat).rows;
                f.end_point = p.scan[seq as usize].num;
                f.jumpline = 0;
            }
        }
    }

    if pos < ctx.m.mod_.len {
        if pos == 0 {
            ctx.p.pos = -1;
        } else {
            ctx.p.pos = pos;
        }
        /* Clear flow vars to prevent old pattern jumps and
         * other junk from executing in the new position. */
        libxmp_reset_flow(ctx);
    }
}

/// Translation of `xmp_set_position()`.
pub(crate) fn xmp_set_position(ctx: &mut ContextData, pos: i32) -> i32 {
    if ctx.state < XMP_STATE_PLAYING {
        return -XMP_ERROR_STATE;
    }

    if pos < 0 || pos >= ctx.m.mod_.len {
        return -XMP_ERROR_INVALID;
    }

    set_position(ctx, pos, 0);

    ctx.p.pos
}

/// Translation of `xmp_stop_module()`.
pub(crate) fn xmp_stop_module(ctx: &mut ContextData) {
    if ctx.state < XMP_STATE_PLAYING {
        return;
    }

    ctx.p.pos = -2;
    libxmp_reset_flow(ctx);
}

/// Translation of `xmp_seek_time()`.
pub(crate) fn xmp_seek_time(ctx: &mut ContextData, time: i32) -> i32 {
    if ctx.state < XMP_STATE_PLAYING {
        return -XMP_ERROR_STATE;
    }

    let mut i = ctx.m.mod_.len - 1;
    while i >= 0 {
        let pat = ctx.m.mod_.xxo[i as usize] as i32;
        if pat >= ctx.m.mod_.pat {
            i -= 1;
            continue;
        }
        if libxmp_get_sequence(ctx, i) != ctx.p.sequence {
            i -= 1;
            continue;
        }
        /* TODO: using rounding to preserve compatibility with
         * the old (bad) int conversion here until this API
         * function can be fixed or replaced. */
        let mut t = ctx.m.xxo_info[i as usize].time;
        clamp(&mut t, 0.0, i32::MAX as f64);
        if time >= t as i32 {
            set_position(ctx, i, 1);
            break;
        }
        i -= 1;
    }
    if i < 0 {
        xmp_set_position(ctx, 0);
    }

    /* Get the correct start row + force the next xmp_play_frame call to
     * do a reposition, which updates properties such as BPM and global
     * volume and normally doesn't happen within the same position: */
    let pos = if ctx.p.pos >= 0 {
        ctx.p.pos
    } else {
        ctx.m.seq_data[ctx.p.sequence as usize].entry_point
    };

    let oinfo = ctx.m.xxo_info[pos as usize];
    ctx.p.flow.jumpline = oinfo.start_row;
    ctx.p.flow.force_reposition = 1;

    /* For the first p->current_time + libxmp_get_frame_time check
     * in xmp_seek_time_frame: */
    ctx.p.current_time = oinfo.time;
    ctx.p.bpm = oinfo.bpm;

    if ctx.p.pos < 0 {
        0
    } else {
        ctx.p.pos
    }
}

/// Translation of `xmp_set_player()`.
pub(crate) fn xmp_set_player(ctx: &mut ContextData, parm: i32, val: i32) -> i32 {
    let mut ret = -XMP_ERROR_INVALID;

    if parm == XMP_PLAYER_SMPCTL || parm == XMP_PLAYER_DEFPAN {
        /* these should be set before loading the module */
        if ctx.state >= XMP_STATE_LOADED {
            return -XMP_ERROR_STATE;
        }
    } else if parm == XMP_PLAYER_VOICES {
        /* these should be set before start playing */
        if ctx.state >= XMP_STATE_PLAYING {
            return -XMP_ERROR_STATE;
        }
    } else if ctx.state < XMP_STATE_PLAYING {
        return -XMP_ERROR_STATE;
    }

    match parm {
        XMP_PLAYER_AMP => {
            if (0..=3).contains(&val) {
                ctx.s.amplify = val;
                ret = 0;
            }
        }
        XMP_PLAYER_MIX => {
            if (-100..=100).contains(&val) {
                ctx.s.mix = val;
                ret = 0;
            }
        }
        XMP_PLAYER_INTERP => {
            if (XMP_INTERP_NEAREST..=XMP_INTERP_SPLINE).contains(&val) {
                ctx.s.interp = val;
                ret = 0;
            }
        }
        XMP_PLAYER_DSP => {
            ctx.s.dsp = val;
            ret = 0;
        }
        XMP_PLAYER_FLAGS => {
            ctx.p.player_flags = val;
            ret = 0;
        }

        /* 4.1 */
        XMP_PLAYER_CFLAGS => {
            let vblank = ctx.p.flags & XMP_FLAGS_VBLANK;
            ctx.p.flags = val;
            if vblank != (ctx.p.flags & XMP_FLAGS_VBLANK) {
                libxmp_scan_sequences(ctx);
            }
            ret = 0;
        }
        XMP_PLAYER_SMPCTL => {
            ctx.m.smpctl = val;
            ret = 0;
        }
        XMP_PLAYER_VOLUME => {
            if (0..=200).contains(&val) {
                ctx.p.master_vol = val;
                ret = 0;
            }
        }
        XMP_PLAYER_SMIX_VOLUME => {
            if (0..=200).contains(&val) {
                ctx.p.smix_vol = val;
                ret = 0;
            }
        }

        /* 4.3 */
        XMP_PLAYER_DEFPAN => {
            if (0..=100).contains(&val) {
                ctx.m.defpan = val;
                ret = 0;
            }
        }

        /* 4.4 */
        XMP_PLAYER_MODE => {
            ctx.p.mode = val;
            libxmp_set_player_mode(ctx);
            libxmp_scan_sequences(ctx);
            ret = 0;
        }
        XMP_PLAYER_VOICES => {
            ctx.s.numvoc = val;
        }
        _ => {}
    }

    ret
}

/// Translation of `xmp_get_player()`.
pub(crate) fn xmp_get_player(ctx: &ContextData, parm: i32) -> i32 {
    let p = &ctx.p;
    let m = &ctx.m;
    let s = &ctx.s;
    let mut ret = -XMP_ERROR_INVALID;

    if parm == XMP_PLAYER_SMPCTL || parm == XMP_PLAYER_DEFPAN {
        // can read these at any time
    } else if parm != XMP_PLAYER_STATE && ctx.state < XMP_STATE_PLAYING {
        return -XMP_ERROR_STATE;
    }

    match parm {
        XMP_PLAYER_AMP => ret = s.amplify,
        XMP_PLAYER_MIX => ret = s.mix,
        XMP_PLAYER_INTERP => ret = s.interp,
        XMP_PLAYER_DSP => ret = s.dsp,
        XMP_PLAYER_FLAGS => ret = p.player_flags,

        /* 4.1 */
        XMP_PLAYER_CFLAGS => ret = p.flags,
        XMP_PLAYER_SMPCTL => ret = m.smpctl,
        XMP_PLAYER_VOLUME => ret = p.master_vol,
        XMP_PLAYER_SMIX_VOLUME => ret = p.smix_vol,

        /* 4.2 */
        XMP_PLAYER_STATE => ret = ctx.state,

        /* 4.3 */
        XMP_PLAYER_DEFPAN => ret = m.defpan,

        /* 4.4 */
        XMP_PLAYER_MODE => ret = p.mode,
        XMP_PLAYER_MIXER_TYPE => {
            ret = XMP_MIXER_STANDARD;
            if p.flags & XMP_FLAGS_A500 != 0 && m.is_amiga_mod() {
                if p.filter != 0 {
                    ret = XMP_MIXER_A500F;
                } else {
                    ret = XMP_MIXER_A500;
                }
            }
        }
        XMP_PLAYER_VOICES => ret = s.numvoc,
        _ => {}
    }

    ret
}

/// The sound of the last `xmp_play_frame()` (the frame info's `buffer`,
/// `buffer_size` bytes of it).
pub(crate) fn frame_buffer<'a>(ctx: &'a ContextData, info: &XmpFrameInfo) -> &'a [u8] {
    let n = (info.buffer_size.max(0) as usize).min(ctx.s.buffer.len());
    &ctx.s.buffer[..n]
}
