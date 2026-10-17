// Rust translation of src/scan.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The scan: the sequences of a module, their durations, and the time,
//! tempo and global volume at the start of each order.

/*
 * Sun, 31 May 1998 17:50:02 -0600
 * Reported by ToyKeeper <scriven@CS.ColoState.EDU>:
 * For loop-prevention, I know a way to do it which lets most songs play
 * fine once through even if they have backward-jumps. Just keep a small
 * array (256 bytes, or even bits) of flags, each entry determining if a
 * pattern in the song order has been played. If you get to an entry which
 * is already non-zero, skip to the next song (assuming looping is off).
 */

/*
 * Tue, 6 Oct 1998 21:23:17 +0200 (CEST)
 * Reported by John v/d Kamp <blade_@dds.nl>:
 * scan.c was hanging when it jumps to an invalid restart value.
 * (Fixed by hipolito)
 */

use super::common::*;
use super::effects::*;
use super::far_extras::{far_module_extras, libxmp_far_translate_tempo};
use super::flow::*;
use super::*;

const VBLANK_TIME_THRESHOLD: f64 = 480000.0; /* 8 minutes */

/// Translation of `scan_module()`.
fn scan_module(ctx: &mut ContextData, ep: i32, chain: i32) -> f64 {
    let mut parm: i32;
    let mut row: i32;
    let mut last_row: i32;
    let mut pdelay: i32 = 0;
    let mut f = FlowControl::default();
    let loop_ = [PatternLoop::default(); XMP_MAX_CHANNELS];
    let mut pat: i32;
    let mut st26_speed: i32;
    let mut far_tempo_coarse: i32;
    let mut far_tempo_fine: i32;
    let mut far_tempo_mode: i32;
    /* was 255, but Global trash goes to 318.
     * Real Tracker supports up to 999 rows, increasing again from 512.
     * Higher limit for MEDs, defiance.crybaby.5 has blocks with 2048+ rows. */
    let row_limit = if ctx.m.is_player_mode_med() {
        3200
    } else {
        1024
    };

    if ctx.m.mod_.len == 0 {
        return 0.0;
    }

    {
        let m = &mut ctx.m;
        let mod_ = &m.mod_;
        for i in 0..mod_.len as usize {
            pat = mod_.xxo[i] as i32;
            let n = if pat >= mod_.pat {
                1
            } else if mod_.pattern(pat).rows != 0 {
                mod_.pattern(pat).rows
            } else {
                1
            };
            m.scan_cnt[i][..n as usize].fill(0);
        }
    }

    /* Use a temporary flow_control so the scan can borrow the player's
     * Pattern Loop handler. */
    f.loop_ = loop_.to_vec();
    f.loop_dest = -1;
    f.loop_param = -1;
    f.loop_start = -1;
    f.loop_count = 0;
    f.loop_active_num = 0;
    f.jump = -1;
    f.jumpline = 0;

    let mut gvl = ctx.m.mod_.gvl;
    let mut bpm = ctx.m.mod_.bpm;

    let mut speed = ctx.m.mod_.spd;
    let base_time = ctx.m.rrate as i32;
    st26_speed = 0;
    far_tempo_coarse = 4;
    far_tempo_fine = 0;
    far_tempo_mode = 1;

    if let Some(me) = far_module_extras(&mut ctx.m) {
        far_tempo_coarse = me.coarse_tempo;
        libxmp_far_translate_tempo(
            far_tempo_mode,
            0,
            far_tempo_coarse,
            &mut far_tempo_fine,
            &mut speed,
            &mut bpm,
        );
    }

    let has_marker = ctx.m.has_quirk(QUIRK_MARKER);

    /* By erlk ozlr <erlk.ozlr@gmail.com>
     *
     * xmp doesn't handle really properly the "start" option (-s for the
     * command-line) for DeusEx's .umx files. These .umx files contain
     * several loop "tracks" that never join together. That's how they put
     * multiple musics on each level with a file per level. Each "track"
     * starts at the same order in all files. The problem is that xmp starts
     * decoding the module at order 0 and not at the order specified with
     * the start option. If we have a module that does "0 -> 2 -> 0 -> ...",
     * we cannot play order 1, even with the supposed right option.
     *
     * was: ord2 = ord = -1;
     *
     * CM: Fixed by using different "sequences" for each loop or subsong.
     *     Each sequence has its entry point. Sequences don't overlap.
     */
    let mut ord = ep - 1;

    let mut gvol_memory = 0;
    let mut row_count = 0;
    let mut row_count_total = 0;
    let mut frame_count = 0;
    let mut orders_since_last_valid = 0;
    let mut any_valid = 0;
    let mut start_time = 0.0;
    let mut time = 0.0;
    let mut inside_loop = 0;

    'end_module: {
        loop {
            /* Sanity check to prevent getting stuck due to broken patterns. */
            if orders_since_last_valid > 512 {
                break;
            }
            orders_since_last_valid += 1;

            let m = &ctx.m;
            let mod_ = &m.mod_;

            ord += 1;
            if ord as u32 >= mod_.len as u32 {
                if mod_.rst > mod_.len || mod_.xxo(mod_.rst) >= mod_.pat {
                    ord = ep;
                } else if libxmp_get_sequence(ctx, ctx.m.mod_.rst) == chain {
                    ord = ctx.m.mod_.rst;
                } else {
                    ord = ep;
                }

                pat = ctx.m.mod_.xxo[ord as usize] as i32;
                if has_marker && pat == XMP_MARK_END as i32 {
                    break;
                }
            }

            pat = ctx.m.mod_.xxo[ord as usize] as i32;

            /* Allow more complex order reuse only in main sequence */
            if ep != 0 && ctx.p.sequence_control[ord as usize] as i32 != NO_SEQUENCE {
                /* Currently to detect the end of the sequence, the player needs the
                 * end to be a real position and row, so skip invalid and S3M_SKIP.
                 * "amazonas-dynomite mix.it" by Skaven has a sequence (9) where an
                 * S3M_END repeats into an S3M_SKIP.
                 *
                 * Two sequences (7 and 8) in "alien incident - leohou2.s3m" by
                 * Purple Motion share the same S3M_END due to an off-by-one jump,
                 * so check S3M_END here too.
                 */
                if pat >= ctx.m.mod_.pat {
                    if has_marker && pat == XMP_MARK_END as i32 {
                        ord = ctx.m.mod_.len;
                    }
                    continue;
                }
                break;
            }
            ctx.p.sequence_control[ord as usize] = chain as u8;

            /* All invalid patterns skipped, only S3M_END aborts replay */
            if pat >= ctx.m.mod_.pat {
                if has_marker && pat == XMP_MARK_END as i32 {
                    ord = ctx.m.mod_.len;
                    continue;
                }
                continue;
            }

            let m = &mut ctx.m;

            if f.jumpline >= m.mod_.pattern(pat).rows {
                f.jumpline = 0;
            }

            /* Changing patterns may reset loop vars. */
            if m.has_flow_mode(FLOW_LOOP_PATTERN_RESET) {
                f.loop_start = -1;
                f.loop_count = 0;
                for i in 0..m.mod_.chn as usize {
                    f.loop_[i].start = 0;
                    f.loop_[i].count = 0;
                }
            }

            /* Loops can cross pattern boundaries, so check if we're not looping */
            if m.scan_cnt[ord as usize][f.jumpline as usize] != 0 && inside_loop == 0 {
                break;
            }

            /* Only update pattern information if we weren't here before. This also
             * means that we don't update pattern information if we're inside a loop,
             * otherwise a loop containing e.g. a global volume fade can make the
             * pattern start with the wrong volume. (fixes xyce-dans_la_rue.xm replay,
             * see https://github.com/libxmp/libxmp/issues/153 for more details).
             */
            let time_factor = m.time_factor;
            {
                let info = &mut m.xxo_info[ord as usize];
                if info.time < 0.0 {
                    info.gvl = gvl;
                    info.bpm = bpm;
                    info.speed = speed;
                    info.time =
                        time + time_factor * frame_count as f64 * base_time as f64 / bpm as f64;
                    info.st26_speed = st26_speed;
                    info.start_row = f.jumpline;
                }

                if info.start_row == 0 && ord != 0 && ord == ep {
                    start_time =
                        time + time_factor * frame_count as f64 * base_time as f64 / bpm as f64;
                }
            }

            /* Get tracks in advance to speed up the event parsing loop. */
            let chn_count = m.mod_.chn;
            let tracks: Vec<i32> = (0..chn_count)
                .map(|chn| m.mod_.track_num(pat, chn))
                .collect();

            last_row = m.mod_.pattern(pat).rows;
            row = f.jumpline;
            f.jumpline = 0;
            while row < last_row {
                /* Prevent crashes caused by large softmixer frames */
                if bpm < XMP_MIN_BPM {
                    bpm = XMP_MIN_BPM;
                }

                /* Date: Sat, 8 Sep 2007 04:01:06 +0200
                 * Reported by Zbigniew Luszpinski <zbiggy@o2.pl>
                 * The scan routine falls into infinite looping and doesn't let
                 * xmp play jos-dr4k.xm.
                 * Claudio's workaround: we'll break infinite loops here.
                 *
                 * Date: Oct 27, 2007 8:05 PM
                 * From: Adric Riedel <adric.riedel@gmail.com>
                 * Jesper Kyd: Global Trash 3.mod (the 'Hardwired' theme) only
                 * plays the first 4:41 of what should be a 10 minute piece.
                 * (...) it dies at the end of position 2F
                 */

                if row_count_total > row_limit {
                    break 'end_module;
                }

                if f.loop_active_num == 0 && m.scan_cnt[ord as usize][row as usize] != 0 {
                    row_count -= 1;
                    break 'end_module;
                }
                m.scan_cnt[ord as usize][row as usize] =
                    m.scan_cnt[ord as usize][row as usize].wrapping_add(1);
                orders_since_last_valid = 0;
                any_valid = 1;

                /* If the scan count for this row overflows, break.
                 * A scan count of 0 will help break this loop in playback (storlek_11.it).
                 */
                if m.scan_cnt[ord as usize][row as usize] == 0 {
                    break 'end_module;
                }

                pdelay = 0;

                for chn in 0..chn_count {
                    let track = m.mod_.track(tracks[chn as usize]);
                    if row >= track.rows {
                        continue;
                    }

                    /* event = &EVENT(mod->xxo[ord], chn, row); */
                    let event = track.event[row as usize];

                    let mut f1 = event.fxt as i32;
                    let p1 = event.fxp as i32;
                    let mut f2 = event.f2t as i32;
                    let p2 = event.f2p as i32;

                    if f1 == 0 && f2 == 0 {
                        continue;
                    }

                    if f1 == FX_GLOBALVOL as i32 || f2 == FX_GLOBALVOL as i32 {
                        gvl = if f1 == FX_GLOBALVOL as i32 { p1 } else { p2 };
                        gvl = if gvl > m.gvolbase {
                            m.gvolbase
                        } else if gvl < 0 {
                            0
                        } else {
                            gvl
                        };
                    }

                    /* Process fine global volume slide */
                    if f1 == FX_GVOL_SLIDE as i32 || f2 == FX_GVOL_SLIDE as i32 {
                        parm = if f1 == FX_GVOL_SLIDE as i32 { p1 } else { p2 };

                        loop {
                            // process_gvol:
                            if parm != 0 {
                                gvol_memory = parm;
                                let h = msn(parm);
                                let l = lsn(parm);

                                if m.has_quirk(QUIRK_FINEFX) {
                                    if l == 0xf && h != 0 {
                                        gvl += h;
                                    } else if h == 0xf && l != 0 {
                                        gvl -= l;
                                    } else if m.quirk & QUIRK_VSALL != 0 {
                                        gvl += (h - l) * speed;
                                    } else {
                                        gvl += (h - l) * (speed - 1);
                                    }
                                } else if m.quirk & QUIRK_VSALL != 0 {
                                    gvl += (h - l) * speed;
                                } else {
                                    gvl += (h - l) * (speed - 1);
                                }
                            } else {
                                parm = gvol_memory;
                                if parm != 0 {
                                    continue;
                                }
                            }
                            break;
                        }
                    }

                    /* Some formats can have two FX_SPEED effects, and both need
                     * to be checked. Slot 2 is currently handled first. */
                    for i in 0..2 {
                        parm = if i != 0 { p1 } else { p2 };
                        if (if i != 0 { f1 } else { f2 }) != FX_SPEED as i32 || parm == 0 {
                            continue;
                        }
                        frame_count += row_count * speed;
                        row_count = 0;
                        if m.has_quirk(QUIRK_NOBPM)
                            || ctx.p.flags & XMP_FLAGS_VBLANK != 0
                            || parm < 0x20
                        {
                            speed = parm;
                            st26_speed = 0;
                        } else {
                            time +=
                                m.time_factor * frame_count as f64 * base_time as f64 / bpm as f64;
                            frame_count = 0;
                            bpm = parm;
                        }
                    }

                    if f1 == FX_SPEED_CP as i32 {
                        f1 = FX_S3M_SPEED as i32;
                    }
                    if f2 == FX_SPEED_CP as i32 {
                        f2 = FX_S3M_SPEED as i32;
                    }

                    /* ST2.6 speed processing */

                    if f1 == FX_ICE_SPEED as i32 && p1 != 0 {
                        if lsn(p1) != 0 {
                            st26_speed = (msn(p1) << 8) | lsn(p1);
                        } else {
                            st26_speed = msn(p1);
                        }
                    }

                    /* FAR tempo processing */

                    if f1 == FX_FAR_TEMPO as i32 || f1 == FX_FAR_F_TEMPO as i32 {
                        let mut far_speed = 0;
                        let mut far_bpm = 0;
                        let mut fine_change = 0;
                        if f1 == FX_FAR_TEMPO as i32 {
                            if msn(p1) != 0 {
                                far_tempo_mode = msn(p1) - 1;
                            } else {
                                far_tempo_coarse = lsn(p1);
                            }
                        }
                        if f1 == FX_FAR_F_TEMPO as i32 {
                            if msn(p1) != 0 {
                                far_tempo_fine += msn(p1);
                                fine_change = msn(p1);
                            } else if lsn(p1) != 0 {
                                far_tempo_fine -= lsn(p1);
                                fine_change = -lsn(p1);
                            } else {
                                far_tempo_fine = 0;
                            }
                        }
                        if libxmp_far_translate_tempo(
                            far_tempo_mode,
                            fine_change,
                            far_tempo_coarse,
                            &mut far_tempo_fine,
                            &mut far_speed,
                            &mut far_bpm,
                        ) == 0
                        {
                            frame_count += row_count * speed;
                            row_count = 0;
                            time +=
                                m.time_factor * frame_count as f64 * base_time as f64 / bpm as f64;
                            frame_count = 0;
                            speed = far_speed;
                            bpm = far_bpm;
                        }
                    }

                    /* ULT tempo processing */

                    if f1 == FX_ULT_TEMPO as i32 || f2 == FX_ULT_TEMPO as i32 {
                        let mut parm2 = 0;
                        parm = 0;
                        if f2 == FX_ULT_TEMPO as i32 {
                            if p2 == 0 {
                                parm = 6;
                                parm2 = 125;
                            } else if p2 < 0x30 {
                                parm = p2;
                            } else {
                                parm2 = p2;
                            }
                        }
                        if f1 == FX_ULT_TEMPO as i32 {
                            if p1 == 0 {
                                parm = 6;
                                parm2 = 125;
                            } else if p1 < 0x30 {
                                parm = p1;
                            } else {
                                parm2 = p1;
                            }
                        }
                        frame_count += row_count * speed;
                        row_count = 0;
                        if parm > 0 {
                            speed = parm;
                            st26_speed = 0;
                        }
                        if parm2 > 0 {
                            time +=
                                m.time_factor * frame_count as f64 * base_time as f64 / bpm as f64;
                            frame_count = 0;
                            bpm = parm2;
                        }
                    }

                    if (f1 == FX_S3M_SPEED as i32 && p1 != 0)
                        || (f2 == FX_S3M_SPEED as i32 && p2 != 0)
                    {
                        parm = if f1 == FX_S3M_SPEED as i32 { p1 } else { p2 };
                        if parm > 0 {
                            frame_count += row_count * speed;
                            row_count = 0;
                            speed = parm;
                            st26_speed = 0;
                        }
                    }

                    if (f1 == FX_S3M_BPM as i32 && p1 != 0) || (f2 == FX_S3M_BPM as i32 && p2 != 0)
                    {
                        parm = if f1 == FX_S3M_BPM as i32 { p1 } else { p2 };
                        if parm >= XMP_MIN_BPM {
                            frame_count += row_count * speed;
                            row_count = 0;
                            time +=
                                m.time_factor * frame_count as f64 * base_time as f64 / bpm as f64;
                            frame_count = 0;
                            bpm = parm;
                        }
                    }

                    if (f1 == FX_IT_BPM as i32 && p1 != 0) || (f2 == FX_IT_BPM as i32 && p2 != 0) {
                        parm = if f1 == FX_IT_BPM as i32 { p1 } else { p2 };
                        frame_count += row_count * speed;
                        row_count = 0;
                        time += m.time_factor * frame_count as f64 * base_time as f64 / bpm as f64;
                        frame_count = 0;

                        if msn(parm) == 0 {
                            time += m.time_factor * base_time as f64 / bpm as f64;
                            for _ in 1..speed {
                                bpm -= lsn(parm);
                                if bpm < 0x20 {
                                    bpm = 0x20;
                                }
                                time += m.time_factor * base_time as f64 / bpm as f64;
                            }

                            /* remove one row at final bpm */
                            time -= m.time_factor * speed as f64 * base_time as f64 / bpm as f64;
                        } else if msn(parm) == 1 {
                            time += m.time_factor * base_time as f64 / bpm as f64;
                            for _ in 1..speed {
                                bpm += lsn(parm);
                                if bpm > 0xff {
                                    bpm = 0xff;
                                }
                                time += m.time_factor * base_time as f64 / bpm as f64;
                            }

                            /* remove one row at final bpm */
                            time -= m.time_factor * speed as f64 * base_time as f64 / bpm as f64;
                        } else {
                            bpm = parm;
                        }
                    }

                    if f1 == FX_IT_ROWDELAY as i32 {
                        /* Don't allow the scan count for this row to overflow here. */
                        let x = m.scan_cnt[ord as usize][row as usize] as i32 + (p1 & 0x0f);
                        m.scan_cnt[ord as usize][row as usize] = x.min(255) as u8;
                        frame_count += (p1 & 0x0f) * speed;
                    }

                    if f1 == FX_IT_BREAK as i32 || f2 == FX_IT_BREAK as i32 {
                        parm = if f1 == FX_IT_BREAK as i32 { p1 } else { p2 };
                        libxmp_process_pattern_break(m, &mut f, parm);
                    }

                    if f1 == FX_JUMP as i32 || f2 == FX_JUMP as i32 {
                        libxmp_process_pattern_jump(
                            m,
                            &mut f,
                            if f1 == FX_JUMP as i32 { p1 } else { p2 },
                        );
                        if f.pbreak != 0 {
                            /* prevent infinite loop, see OpenMPT PatLoop-Various.xm */
                            inside_loop = 0;
                        }
                    }

                    if f1 == FX_BREAK as i32 || f2 == FX_BREAK as i32 {
                        parm = if f1 == FX_BREAK as i32 { p1 } else { p2 };
                        parm = 10 * msn(parm) + lsn(parm);
                        libxmp_process_pattern_break(m, &mut f, parm);
                    }

                    /* Archimedes line jump */
                    if f1 == FX_LINE_JUMP as i32 || f2 == FX_LINE_JUMP as i32 {
                        libxmp_process_line_jump(
                            &mut f,
                            ord,
                            if f1 == FX_LINE_JUMP as i32 { p1 } else { p2 },
                        );
                    }

                    if f1 == FX_EXTENDED as i32 || f2 == FX_EXTENDED as i32 {
                        parm = if f1 == FX_EXTENDED as i32 { p1 } else { p2 };

                        if (parm >> 4) == EX_PATT_DELAY
                            && (m.read_event_type != READ_EVENT_ST3 || pdelay == 0)
                        {
                            pdelay = parm & 0x0f;
                        }

                        if (parm >> 4) == EX_PATTERN_LOOP {
                            libxmp_process_pattern_loop(m, &mut f, chn, row, lsn(parm));

                            /* Attempt to detect the inside of a loop.
                             * TODO: this won't detect all cases. */
                            if lsn(parm) > 0 && f.loop_dest < 0 {
                                inside_loop = 0;
                            } else if lsn(parm) == 0 {
                                inside_loop = 1;
                            }
                        }
                    }

                    /* OctaMED pattern delay */
                    if f1 == FX_PATT_DELAY as i32 {
                        pdelay += p1;
                    }
                    if f2 == FX_PATT_DELAY as i32 {
                        pdelay += p2;
                    }
                }

                if pdelay > 0 {
                    frame_count += pdelay * speed;
                }

                f.loop_param = -1;
                if f.loop_dest >= 0 {
                    /* -1 as it will be incremented immediately by the loop. */
                    row = f.loop_dest - 1;
                    f.loop_dest = -1;
                }

                if f.pbreak != 0 {
                    f.pbreak = 0;
                    last_row = 0;
                }

                if st26_speed != 0 {
                    frame_count += row_count * speed;
                    row_count = 0;
                    if st26_speed & 0x10000 != 0 {
                        speed = (st26_speed & 0xff00) >> 8;
                    } else {
                        speed = st26_speed & 0xff;
                    }
                    st26_speed ^= 0x10000;
                }

                row += 1;
                row_count += 1;
                row_count_total += 1;
            }

            if f.jumpline != 0 && pdelay != 0 {
                f.jumpline += 1;
            }

            if f.jump >= 0 {
                ord = f.jump - 1;
                f.jump = -1;
            }

            frame_count += row_count * speed;
            row_count_total = 0;
            row_count = 0;
        }
        row = f.jumpline;
    }

    // end_module:

    let m = &ctx.m;
    let mod_ = &m.mod_;
    /* Sanity check */
    {
        if any_valid == 0 {
            return -1.0;
        }
        pat = mod_.xxo(ord);
        if pat >= mod_.pat || row >= mod_.pattern(pat).rows {
            row = 0;
        }
    }

    let p = &mut ctx.p;
    // (`ord` is `len` if the scan gave up just after an end marker: upstream
    // reads past its scan counters then; 0 here.)
    p.scan[chain as usize].num = m
        .scan_cnt
        .get(ord as usize)
        .map_or(0, |c| c[row as usize] as i32);
    p.scan[chain as usize].row = row;
    p.scan[chain as usize].ord = ord;

    time -= start_time;
    frame_count += row_count * speed;

    time + m.time_factor * frame_count as f64 * base_time as f64 / bpm as f64
}

/// Translation of `reset_scan_data()`.
fn reset_scan_data(ctx: &mut ContextData) {
    for i in 0..XMP_MAX_MOD_LENGTH {
        ctx.m.xxo_info[i].time = -1.0;
    }
    ctx.p.sequence_control.fill(NO_SEQUENCE as u8);
    ctx.p.scan[0] = ScanData::default();
}

/// Translation of `compare_vblank_scan()`.
fn compare_vblank_scan(ctx: &mut ContextData) {
    /* Calculate both CIA and VBlank time for certain long MODs
     * and pick the more likely (i.e. shorter) one. The same logic
     * works regardless of the initial mode selected--either way,
     * the wrong timing mode usually makes modules MUCH longer. */

    /* Back up the current info to avoid a third scan. */
    let scan_backup = ctx.p.scan[0];
    let info_backup = ctx.m.xxo_info;
    let ctrl_backup = ctx.p.sequence_control;

    reset_scan_data(ctx);

    ctx.m.quirk ^= QUIRK_NOBPM;
    ctx.p.scan[0].time = scan_module(ctx, 0, 0);

    if ctx.p.scan[0].time >= scan_backup.time {
        ctx.m.quirk ^= QUIRK_NOBPM;
        ctx.p.scan[0] = scan_backup;
        ctx.m.xxo_info = info_backup;
        ctx.p.sequence_control = ctrl_backup;
    }
}

/// Translation of `libxmp_get_sequence()`.
pub(crate) fn libxmp_get_sequence(ctx: &ContextData, ord: i32) -> i32 {
    let p = &ctx.p;
    if ord < 0 || ord > ctx.m.mod_.len {
        return NO_SEQUENCE;
    }
    // FIXME (upstream): `ord == len` reads one past the orders, which for
    // a 256-order module is past the array (into `smix_vol`): here, as if
    // that order had no sequence.
    p.sequence_control
        .get(ord as usize)
        .map_or(NO_SEQUENCE, |&v| v as i32)
}

/// Translation of `libxmp_scan_sequences()`.
pub(crate) fn libxmp_scan_sequences(ctx: &mut ContextData) -> i32 {
    let mut temp_ep = [0u8; XMP_MAX_MOD_LENGTH];

    let n = ctx.m.mod_.len.max(1) as usize;
    if ctx.p.scan.len() < n {
        if ctx.p.scan.try_reserve(n - ctx.p.scan.len()).is_err() {
            return -1;
        }
    }
    ctx.p.scan.resize(n, ScanData::default());

    /* Initialize order data to prevent overwrite when a position is used
     * multiple times at different starting points (see janosik.xm).
     */
    reset_scan_data(ctx);

    let mut ep = 0;
    temp_ep[0] = 0;
    ctx.p.scan[0].time = scan_module(ctx, ep, 0);
    let mut seq = 1;

    if ctx.m.compare_vblank != 0
        && ctx.p.flags & XMP_FLAGS_VBLANK == 0
        && ctx.p.scan[0].time >= VBLANK_TIME_THRESHOLD
    {
        compare_vblank_scan(ctx);
    }

    loop {
        /* Scan song starting at given entry point */
        /* Check if any patterns left */
        let len = ctx.m.mod_.len;
        let mut i = 0;
        while i < len {
            if ctx.p.sequence_control[i as usize] as i32 == NO_SEQUENCE {
                break;
            }
            i += 1;
        }
        if i != len && seq < MAX_SEQUENCES {
            /* New entry point */
            ep = i;
            temp_ep[seq] = ep as u8;
            let t = scan_module(ctx, ep, seq as i32);
            ctx.p.scan[seq].time = t;
            if ctx.p.scan[seq].time > 0.0 {
                seq += 1;
            }
        } else {
            break;
        }
    }

    if (seq as i32) < ctx.m.mod_.len {
        ctx.p.scan.truncate(seq);
    }
    let m = &mut ctx.m;
    let p = &mut ctx.p;
    m.num_sequences = seq as i32;

    /* Correct playback time calculation to match new base tempo factor. */
    p.current_time *= m.time_factor / p.scan_time_factor;
    p.scan_time_factor = m.time_factor;

    /* Now place entry points in the public accessible array */
    let mut i = 0;
    while i < m.num_sequences as usize {
        /* API still uses integers for time... */
        let mut time = p.scan[i].time;
        clamp(&mut time, 0.0, i32::MAX as f64);

        m.seq_data[i].entry_point = temp_ep[i] as i32;
        m.seq_data[i].duration = time as i32;
        i += 1;
    }
    /* Wipe the remaining entries so the player doesn't think they're
     * valid e.g. when handling end-of-module markers. */
    while i < MAX_SEQUENCES {
        m.seq_data[i].entry_point = 0;
        m.seq_data[i].duration = 0;
        i += 1;
    }
    /* Correct any zero-length temporary sequences from the scan.
     * There's no "correct" sequence for these, so copy whichever
     * valid sequence precedes the affected position. */
    for i in 0..m.mod_.len as usize {
        if p.sequence_control[i] as i32 >= m.num_sequences {
            p.sequence_control[i] = if i > 0 { p.sequence_control[i - 1] } else { 0 };
        }
    }

    /* If the module is currently playing, rescanning the sequences may
     * invalidate the current sequence. */
    if ctx.state >= XMP_STATE_PLAYING {
        let seq = libxmp_get_sequence(ctx, ctx.p.pos);
        ctx.p.sequence = if seq != NO_SEQUENCE { seq } else { 0 };
    }

    0
}
