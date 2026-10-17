// Rust translation of src/flow.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Pattern loop, jump and break, shared by the player and the scan. (They
//! take the module's data and a `flow_control` where upstream takes the
//! context and one.)

use super::common::*;

/// Which of a pattern loop's start and count variables to use: the
/// channel's, or the global ones.
#[derive(Clone, Copy)]
enum LoopVar {
    Chn,
    Global,
}

/* Process a pattern loop effect with the parameter fxp. A parameter of 0
 * will set the loop target, and a parameter of 1-15 (most formats) or
 * 1-255 (OctaMED) will perform a loop.
 *
 * The compatibility logic for Pattern Loop is complex, so a flow_control
 * argument is taken such that the scan can use this function directly.
 *
 * If the development tests ever start building against effects.c, this
 * can be moved back to effects.c.
 */
/// Translation of `libxmp_process_pattern_loop()`.
pub(crate) fn libxmp_process_pattern_loop(
    m: &ModuleData,
    f: &mut FlowControl,
    chn: i32,
    row: i32,
    fxp: i32,
) {
    let mod_ = &m.mod_;
    let mut start = LoopVar::Chn;
    let mut count = LoopVar::Chn;
    let mut looped = 0;
    let c = chn as usize;

    macro_rules! start {
        () => {
            match start {
                LoopVar::Chn => f.loop_[c].start,
                LoopVar::Global => f.loop_start,
            }
        };
        ($v:expr) => {{
            let v = $v;
            match start {
                LoopVar::Chn => f.loop_[c].start = v,
                LoopVar::Global => f.loop_start = v,
            }
        }};
    }
    macro_rules! count {
        () => {
            match count {
                LoopVar::Chn => f.loop_[c].count,
                LoopVar::Global => f.loop_count,
            }
        };
        ($v:expr) => {{
            let v = $v;
            match count {
                LoopVar::Chn => f.loop_[c].count = v,
                LoopVar::Global => f.loop_count = v,
            }
        }};
    }

    /* Digital Tracker: only the first E60 or E6x is handled per row. */
    if m.has_flow_mode(FLOW_LOOP_FIRST_EFFECT) && f.loop_param >= 0 {
        return;
    }
    f.loop_param = fxp;

    /* Scream Tracker 3, Digital Tracker, Octalyser, and probably others
     * use global loop targets and counts. Later versions of Digital
     * Tracker use a global target but per-track counts. */
    if m.has_flow_mode(FLOW_LOOP_GLOBAL_TARGET) {
        start = LoopVar::Global;
    }
    if m.has_flow_mode(FLOW_LOOP_GLOBAL_COUNT) {
        count = LoopVar::Global;
    }

    if fxp == 0 {
        /* mark start of loop */
        /* Liquid Tracker: M60 is ignored for channels with count >= 1 */
        if m.has_flow_mode(FLOW_LOOP_IGNORE_TARGET) && count!() >= 1 {
            return;
        }
        start!(row);
        if m.has_quirk(QUIRK_FT2BUGS) {
            f.jumpline = row;
        }
    } else {
        /* end of loop */
        if start!() < 0 {
            /* Scream Tracker 3.01b: if SB0 wasn't used, the first
             * SBx used will set the loop target to its row. */
            if m.has_flow_mode(FLOW_LOOP_INIT_SAMEROW) {
                start!(row);
            } else {
                start!(0);
            }
        }

        if count!() != 0 {
            count!(count!() - 1);
            if count!() != 0 {
                f.loop_dest = start!();
                looped = 1;
            } else {
                /* S3M and IT: loop termination advances the
                 * loop target past SBx. */
                if m.has_flow_mode(FLOW_LOOP_END_ADVANCES) {
                    start!(row + 1);
                }
                /* Liquid Tracker cancels any other loop jumps
                 * this row started on loop termination. */
                if m.has_flow_mode(FLOW_LOOP_END_CANCELS) {
                    f.loop_dest = -1;
                }
                f.loop_active_num -= 1;
            }
        } else {
            /* Modplug Tracker: only begin a loop if no
             * other channel is currently looping. */
            if m.has_flow_mode(FLOW_LOOP_ONE_AT_A_TIME) {
                for i in 0..mod_.chn {
                    if i != chn && f.loop_[i as usize].count != 0 {
                        return;
                    }
                }
            }
            count!(fxp);
            f.loop_dest = start!();
            f.loop_active_num += 1;
            looped = 1;
        }
    }

    /* Hacks for loop jumps altering prior position jumps/breaks. */
    if looped != 0 && f.pbreak != 0 {
        /* Many implementations use the same variable for both the
         * jump/break destination row and the loop destination row. */
        if m.has_flow_mode(FLOW_LOOP_SHARED_BREAK) {
            f.jumpline = f.loop_dest;
        }
        /* Various players e.g. ST3, IT will block prior breaks. */
        if m.has_flow_mode(FLOW_LOOP_UNSET_BREAK) && f.jump < 0 {
            f.pbreak = 0;
        }
        /* Various players e.g. ST3, IT will block prior jumps. */
        if m.has_flow_mode(FLOW_LOOP_UNSET_JUMP) && f.jump >= 0 {
            f.pbreak = 0;
            f.jump = -1;
        }
    }
}

/* Process a pattern jump effect with the parameter fxp.
 * This function will not actually perform the jump, but it will
 * prepare the flow variables to perform a jump (unless prevented).
 */
/// Translation of `libxmp_process_pattern_jump()`.
pub(crate) fn libxmp_process_pattern_jump(m: &ModuleData, f: &mut FlowControl, fxp: i32) {
    /* Some formats and trackers e.g. S3M, Modplug Tracker 1.16 will
     * prevent jumps from being executed when a loop jump occurs. */
    if m.has_flow_mode(FLOW_LOOP_DELAY_JUMP) && f.loop_dest >= 0 {
        return;
    }

    f.pbreak = 1;
    f.jump = fxp;
    if !m.has_flow_mode(FLOW_JUMP_NO_ROW_SET) {
        /* Effect B resets effect D in lower channels.
         * This does not apply to ST3, IT, ModPlug. */
        f.jumpline = 0;
    }
}

/* Process a pattern break effect with the parameter fxp.
 * This function will not actually perform the jump, but it will
 * prepare the flow variables to perform a jump (unless prevented).
 */
/// Translation of `libxmp_process_pattern_break()`.
pub(crate) fn libxmp_process_pattern_break(m: &ModuleData, f: &mut FlowControl, fxp: i32) {
    /* Some formats and trackers e.g. S3M, IT 2.00+, Modplug Tracker 1.16
     * will prevent breaks from being executed when a loop jump occurs. */
    if m.has_flow_mode(FLOW_LOOP_DELAY_BREAK) && f.loop_dest >= 0 {
        return;
    }

    f.pbreak = 1;
    f.jumpline = fxp;
}

/* Process a line jump within current position `ord` to row `fxp`.
 * This function will not actually perform the jump, but it will
 * prepare the flow variables to perform a jump (unless prevented).
 */
/// Translation of `libxmp_process_line_jump()`.
pub(crate) fn libxmp_process_line_jump(f: &mut FlowControl, ord: i32, fxp: i32) {
    /* In Digital Symphony, this can be combined with position jump
     * (like pattern break) and overrides the pattern break line in
     * lower channels. */
    if f.pbreak == 0 {
        f.pbreak = 1;
        f.jump = ord;
    }
    f.jumpline = fxp;
}
