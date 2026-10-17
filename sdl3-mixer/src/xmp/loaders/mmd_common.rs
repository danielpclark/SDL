// Rust translation of src/loaders/mmd_common.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * Common functions for MMD0/1 and MMD2/3 loaders
 * Tempo fixed by Francis Russell
 */

//! What the MED and OctaMED loaders share: effects, tempos and
//! instruments.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::med_extras::*;
use super::super::path::*;
use super::super::*;
use super::common::*;
use super::med::*;
use super::sample::libxmp_load_sample;
use super::*;

/* (upstream keeps the instrument type names here for debug builds) */

/// Translation of `mmd_convert_tempo()`.
pub(crate) fn mmd_convert_tempo(mut tempo: i32, bpm_on: bool, med_8ch: bool) -> i32 {
    const TEMPOS_COMPAT: [i32; 10] = [195, 97, 65, 49, 39, 32, 28, 24, 22, 20];
    const TEMPOS_8CH: [i32; 10] = [179, 164, 152, 141, 131, 123, 116, 110, 104, 99];

    if tempo > 0 {
        /* From the OctaMEDv4 documentation:
         *
         * In 8-channel mode, you can control the playing speed more
         * accurately (to techies: by changing the size of the mix buffer).
         * This can be done with the left tempo gadget (values 1-10; the
         * lower, the faster). Values 11-240 are equivalent to 10.
         *
         * NOTE: the tempos used for this are directly from OctaMED
         * Soundstudio 2, but in older versions the playback speeds
         * differed slightly between NTSC and PAL. This table seems to
         * have been intended to be a compromise between the two.
         */
        if med_8ch {
            tempo = if tempo > 10 { 10 } else { tempo };
            return TEMPOS_8CH[(tempo - 1) as usize];
        }
        /* Tempos 1-10 in tempo mode are compatibility tempos that
         * approximate Soundtracker speeds.
         */
        if tempo <= 10 && !bpm_on {
            return TEMPOS_COMPAT[(tempo - 1) as usize];
        }
    }
    tempo
}

/// Translation of `mmd_xlat_fx()`.
pub(crate) fn mmd_xlat_fx(
    event: &mut XmpEvent,
    bpm_on: bool,
    _bpmlen: i32,
    med_8ch: bool,
    hexvol: bool,
) {
    match event.fxt {
        0x00 => {
            /* ARPEGGIO 00
             * Changes the pitch six times between three different
             * pitches during the duration of the note. It can create a
             * chord sound or other special effect. Arpeggio works better
             * with some instruments than others.
             */
        }
        0x01 => {
            /* SLIDE UP 01
             * This slides the pitch of the current track up. It decreases
             * the period of the note by the amount of the argument on each
             * timing pulse. OctaMED-Pro can create slides automatically,
             * but you may want to use this function for special effects.
             * Note: a param of 0 does nothing and should be ignored.
             */
            if event.fxp == 0 {
                event.fxt = 0;
            }
        }
        0x02 => {
            /* SLIDE DOWN 02
             * The same as SLIDE UP, but it slides down.
             * Note: a param of 0 does nothing and should be ignored.
             */
            if event.fxp == 0 {
                event.fxt = 0;
            }
        }
        0x03 => {
            /* PORTAMENTO 03
             * Makes precise sliding easy.
             */
        }
        0x04 => {
            /* VIBRATO 04
             * The left half of the argument is the vibrato speed, the
             * right half is the depth. If the numbers are zeros, the
             * previous speed and depth are used.
             */
            /* Note: this is twice as deep as the Protracker vibrato */
            event.fxt = FX_VIBRATO2;
        }
        0x05 | 0x06 | 0x07 => {
            /* SLIDE + FADE 05
             * ProTracker compatible. This command is the combination of
             * commands 0300 and 0Dxx. The argument is the fade speed.
             * The slide will continue during this command.
             */
            /* fall-through */
            /* VIBRATO + FADE 06
             * ProTracker compatible. Combines commands 0400 and 0Dxx.
             * The argument is the fade speed. The vibrato will continue
             * during this command.
             */
            /* fall-through */
            /* TREMOLO 07
             * ProTracker compatible.
             * This command is a kind of "volume vibrato". The left
             * number is the speed of the tremolo, and the right one is
             * the depth. The depth must be quite high before the effect
             * is audible.
             */
        }
        0x08 => {
            /* HOLD and DECAY 08
             * This command must be on the same line as the note. The
             * left half of the argument determines the decay and the
             * right half the hold.
             */
            event.fxt = FX_MED_HOLD;
        }
        0x09 => {
            /* SECONDARY TEMPO 09
             * This sets the secondary tempo (the number of timing
             * pulses per note). The argument must be from 01 to 20 (hex).
             */
            if event.fxp >= 0x01 && event.fxp <= 0x20 {
                event.fxt = FX_SPEED;
            } else {
                event.fxt = 0;
                event.fxp = 0;
            }
        }
        0x0a | 0x0b | 0x0c => {
            /* 0A not mentioned but it's Protracker-compatible */
            /* fall-through */
            /* POSITION JUMP 0B
             * The song plays up to this command and then jumps to
             * another position in the play sequence. The song then
             * loops from the point jumped to, to the end of the song
             * forever. The purpose is to allow for introductions that
             * play only once.
             */
            /* fall-through */
            /* SET VOLUME 0C
             * Overrides the default volume of an instrument.
             */
            if !hexvol {
                // FIXME (upstream): the decimal conversion shifts by 8 and
                // masks with 0xff, which leaves the 8 bit parameter as it
                // is.
                let p = event.fxp as i32;
                event.fxp = ((p >> 8) * 10 + (p & 0xff)) as u8;
            }
        }
        0x0d => {
            /* VOLUME SLIDE 0D
             * Smoothly slides the volume up or down. The left half of
             * the argument increases the volume. The right decreases it.
             */
            event.fxt = FX_VOLSLIDE;
        }
        0x0e => {
            /* SYNTH JUMP 0E
             * When used with synthetic or hybrid instruments, it
             * triggers a jump in the Waveform Sequence List. The argument
             * is the jump destination (line no).
             */
            event.fxt = 0;
            event.fxp = 0;
        }
        0x0f => {
            /* MISCELLANEOUS 0F
             * The effect depends upon the value of the argument.
             */
            if event.fxp == 0x00 {
                /* Jump to next block */
                event.fxt = FX_BREAK;
            } else if event.fxp <= 0xf0 {
                event.fxt = FX_S3M_BPM;
                event.fxp = mmd_convert_tempo(event.fxp as i32, bpm_on, med_8ch) as u8;
            } else {
                match event.fxp {
                    0xf1 => {
                        /* Play note twice */
                        event.fxt = FX_EXTENDED;
                        event.fxp = ((EX_RETRIG << 4) | 3) as u8;
                    }
                    0xf2 => {
                        /* Delay note */
                        event.fxt = FX_EXTENDED;
                        event.fxp = ((EX_DELAY << 4) | 3) as u8;
                    }
                    0xf3 => {
                        /* Play note three times */
                        /* Actually just retriggers every 2 ticks, except
                         * for a bug in OctaMED <=4.00 where it will retrigger
                         * on (tick & 7) >= 2 (TODO: verify). */
                        event.fxt = FX_MED_RETRIG;
                        event.fxp = 0x02;
                    }
                    0xf8 /* Turn filter off */
                    | 0xf9 /* Turn filter on */
                    | 0xfa /* MIDI pedal on */
                    | 0xfb /* MIDI pedal off */
                    | 0xfd /* Set pitch */
                    | 0xfe /* End of song */ => {
                        event.fxt = 0;
                        event.fxp = 0;
                    }
                    0xff => {
                        /* Turn note off */
                        event.fxt = 0;
                        event.fxp = 0;
                        event.note = XMP_KEY_CUT;
                    }
                    _ => {
                        event.fxt = 0;
                        event.fxp = 0;
                    }
                }
            }
        }
        0x11 => {
            /* SLIDE PITCH UP (only once) 11
             * Equivalent to ProTracker command E1x.
             * Lets you control the pitch with great accuracy. This
             * command changes only this occurrence of the note.
             * Note: a param of 0 does nothing and should be ignored.
             */
            event.fxt = FX_F_PORTA_UP;
        }
        0x12 => {
            /* SLIDE DOWN (only once) 12
             * Equivalent to ProTracker command E2x.
             * Note: a param of 0 does nothing and should be ignored.
             */
            event.fxt = FX_F_PORTA_DN;
        }
        0x14 => {
            /* VIBRATO 14
             * ProTracker compatible. This is similar to command 04
             * except the depth is halved, to give greater accuracy.
             */
            event.fxt = FX_VIBRATO;
        }
        0x15 => {
            /* SET FINETUNE 15
             * Set a finetune value for a note, overrides the default
             * fine tune value of the instrument. Negative numbers must
             * be entered as follows:
             *   -1 => FF    -3 => FD    -5 => FB    -7 => F9
             *   -2 => FE    -4 => FC    -6 => FA    -8 => F8
             */
            event.fxt = FX_FINETUNE;
            event.fxp = event.fxp.wrapping_add(8) << 4;
        }
        0x16 => {
            /* LOOP 16
             * Creates a loop within a block. 1600 marks the beginning
             * of the loop.  The next occurrence of the 16 command
             * designates the number of loops. Same as ProTracker E6x.
             */
            event.fxt = FX_EXTENDED;
            if event.fxp > 0x0f {
                event.fxp = 0x0f;
            }
            event.fxp |= 0x60;
        }
        0x18 => {
            /* STOP NOTE 18
             * Cuts the note by zeroing the volume at the pulse specified
             * in the argument value. This is the same as ProTracker
             * command ECx.
             */
            event.fxt = FX_EXTENDED;
            if event.fxp > 0x0f {
                event.fxp = 0x0f;
            }
            event.fxp |= 0xc0;
        }
        0x19 => {
            /* SET SAMPLE START OFFSET
             * Same as ProTracker command 9.
             * When playing a sample, this command sets the starting
             * offset (at steps of $100 = 256 bytes). Useful for speech
             * samples.
             */
            event.fxt = FX_OFFSET;
        }
        0x1a => {
            /* SLIDE VOLUME UP ONCE
             * Only once ProTracker command EAx. Lets volume slide
             * slowly once per line.
             * Note: a param of 0 does nothing and should be ignored.
             */
            event.fxt = if event.fxp != 0 { FX_F_VSLIDE_UP } else { 0 };
        }
        0x1b => {
            /* SLIDE VOLUME DOWN ONCE
             * Only once ProTracker command EBx.
             * Note: a param of 0 does nothing and should be ignored.
             */
            event.fxt = if event.fxp != 0 { FX_F_VSLIDE_DN } else { 0 };
        }
        0x1d => {
            /* JUMP TO NEXT BLOCK 1D
             * Jumps to the next line in the PLAY SEQUENCE LIST at the
             * specified line. ProTracker command D. This command is
             * like F00, except that you can specify the line number of
             * the first line to be played. The line number must be
             * specified in HEX.
             */
            event.fxt = FX_BREAK;
        }
        0x1e => {
            /* PLAY LINE x TIMES 1E
             * Plays only commands, notes not replayed. ProTracker
             * pattern delay.
             */
            event.fxt = FX_PATT_DELAY;
        }
        0x1f => {
            /* Command 1F: NOTE DELAY AND RETRIGGER
             * (Protracker commands EC and ED)
             * Gives you accurate control over note playing. You can
             * delay the note any number of ticks, and initiate fast
             * retrigger. Level 1 = note delay value, level 2 = retrigger
             * value.
             * Unlike FF1/FF3, this does nothing on a line with no note.
             */
            if event.fxp != 0 && event.note != 0 {
                event.fxt = FX_MED_RETRIG;
            } else {
                event.fxt = 0;
                event.fxp = 0;
            }
        }
        0x20 => {
            /* Command 20: REVERSE SAMPLE / RELATIVE SAMPLE OFFSET
             * With command level $00, the sample is reversed. With other
             * levels, it changes the sample offset, just like command 19,
             * except the command level is the new offset relative to the
             * current sample position being played.
             * Note: 20 00 only works on the same line as a new note.
             */
            if event.fxp == 0 && event.note != 0 {
                event.fxt = FX_REVERSE;
                event.fxp = 1;
            } else {
                event.fxt = 0;
                event.fxp = 0;
            }
        }
        0x2e => {
            /* Command 2E: SET TRACK PANNING
             * Allows track panning to be changed during play. The track
             * on which the player command appears is the track affected.
             * The command level is in signed hex: $F0 to $10 = -16 to 16
             * decimal.
             */
            if event.fxp >= 0xf0 || event.fxp <= 0x10 {
                let mut fxp = event.fxp as i8 as i32 + 16;
                fxp <<= 3;
                if fxp == 0x100 {
                    fxp -= 1;
                }
                event.fxt = FX_SETPAN;
                event.fxp = fxp as u8;
            }
        }
        _ => {
            event.fxt = 0;
            event.fxp = 0;
        }
    }
}

/// Translation of `struct mmd_instrument_info`.
#[derive(Clone, Copy, Default)]
struct MmdInstrumentInfo {
    length: u32,
    rep: u32,
    replen: u32,
    sampletrans: i32,
    synthtrans: i32,
    flg: i32,
    enable: bool,
}

/* Interpret loop/flag parameters for sampled instruments (sample, hybrid, IFF).
 * This is common code to avoid replicating this mess in each loader. */
/// Translation of `mmd_load_instrument_common()`.
fn mmd_load_instrument_common(
    info: &mut MmdInstrumentInfo,
    instr: Option<&InstrHdr>,
    expdata: &Mmd0Exp,
    exp_smp: &InstrExt,
    sample: &Mmd0Sample,
    ver: i32,
) {
    info.enable = true;
    info.flg = 0;
    if ver >= 2 && expdata.s_ext_entrsz >= 8 {
        /* MMD2+ instrument flags */
        let instr_flags = exp_smp.instr_flags;

        if instr_flags & SSFLG_LOOP != 0 {
            info.flg |= XMP_SAMPLE_LOOP;
        }
        if instr_flags & SSFLG_PINGPONG != 0 {
            info.flg |= XMP_SAMPLE_LOOP_BIDIR;
        }
        if instr_flags & SSFLG_DISABLED != 0 {
            info.enable = false;
        }
    } else if sample.replen > 1 {
        info.flg |= XMP_SAMPLE_LOOP;
    }

    info.sampletrans = 36 + sample.strans as i32;
    info.synthtrans = 12 + sample.strans as i32;

    if let Some(instr) = instr {
        let sample_type = instr.type_ as i32 & !(S_16 | MD16 | STEREO);

        if (ver >= 3 && sample_type == 0) || sample_type == 7 {
            /* Mix mode transposes samples down two octaves.
             * This does not apply to octave samples or synths.
             * ExtSamples (7) are transposed regardless. */
            info.sampletrans -= 24;
        }

        info.length = instr.length;

        /* OctaMED Soundstudio always loads these values but only uses
         * them in mixing mode (MMD3); prior versions ignore them.
         * MED Soundstudio 2 always uses these fields when found in
         * MMD0-2 modules, but can only save MMD3 modules. */
        if ver >= 3 && expdata.s_ext_entrsz >= 18 {
            info.rep = exp_smp.long_repeat;
            info.replen = exp_smp.long_replen;
        } else {
            info.rep = (sample.rep as u32) << 1;
            info.replen = (sample.replen as u32) << 1;
        }

        if instr.type_ as i32 & S_16 != 0 {
            info.flg |= XMP_SAMPLE_16BIT;
            /* Length is (bytes / channels) but the
             * loop is measured in sample frames. */
            info.length >>= 1;
        }

        /* STEREO means that this is a stereo sample. The sample
         * is not interleaved. The left channel comes first,
         * followed by the right channel. Important: Length
         * specifies the size of one channel only! The actual memory
         * usage for both samples is length * 2 bytes.
         */
        if instr.type_ as i32 & STEREO != 0 {
            info.flg |= XMP_SAMPLE_STEREO;
        }
    }
}

/* Compatibility for MED Soundstudio v2 default pitch events.
 * For single-octave samples and synthetics, MED mix mode note 0x01
 * plays the note number stored in the InstrExt default pitch field.
 * Mix mode events are currently transposed up an octave and are offset
 * down by 1 for the instrument map, hence index 12.
 *
 * See med.h for more info.
 */
/// Translation of `mmd_set_default_pitch_note()`.
fn mmd_set_default_pitch_note(xxi: &mut XmpInstrument, exp_smp: &InstrExt, ver: i32) {
    if ver >= 3 {
        let mut note = MMD3_DEFAULT_NOTE;

        if exp_smp.default_pitch != 0 {
            note = exp_smp.default_pitch as i32 - 1;
        }

        if note >= 0 && (note as usize) < XMP_MAX_KEYS {
            xxi.map[12].xpo = note as i8;
        }
    }
}

/// Translation of `mmd_alloc_tables()`.
pub(crate) fn mmd_alloc_tables(m: &mut ModuleData, i: i32, synth: &SynthInstr) -> i32 {
    let Some(me) = med_module_extras_mut(m) else {
        return -1;
    };
    let i = i as usize;
    if i >= me.vol_table.len() || i >= me.wav_table.len() {
        return -1;
    }

    let vlen = synth.voltbllen as usize;
    let Ok(mut vol) = try_vec(vlen, 0u8) else {
        return -1;
    };
    vol.copy_from_slice(&synth.voltbl[..vlen]);
    me.vol_table[i] = Some(vol);

    let wlen = synth.wftbllen as usize;
    let Ok(mut wav) = try_vec(wlen, 0u8) else {
        me.vol_table[i] = None;
        return -1;
    };
    wav.copy_from_slice(&synth.wftbl[..wlen]);
    me.wav_table[i] = Some(wav);

    0
}

/// The common reading of `struct SynthInstr` past its length and type.
fn read_synth_header(synth: &mut SynthInstr, f: &mut HioHandle<'_>) {
    synth.defaultdecay = hio_read8(f);
    hio_seek(f, 3, SEEK_CUR);
    synth.rep = hio_read16b(f);
    synth.replen = hio_read16b(f);
    synth.voltbllen = hio_read16b(f);
    synth.wftbllen = hio_read16b(f);
    synth.volspeed = hio_read8(f);
    synth.wfspeed = hio_read8(f);
    synth.wforms = hio_read16b(f);
    hio_read(&mut synth.voltbl, 1, 128, f);
    hio_read(&mut synth.wftbl, 1, 128, f);
}

/// `pos - 6 + synth->wf[j]` (an int plus an unsigned 32 bit number).
#[inline]
fn wf_pos(pos: i32, wf: u32) -> i64 {
    (pos.wrapping_sub(6) as u32).wrapping_add(wf) as i64
}

/// Translation of `mmd_load_hybrid_instrument()`.
#[allow(clippy::too_many_arguments)]
fn mmd_load_hybrid_instrument(
    f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
    mut smp_idx: i32,
    synth: &mut SynthInstr,
    expdata: &Mmd0Exp,
    exp_smp: &InstrExt,
    sample: &Mmd0Sample,
    ver: i32,
) -> i32 {
    let iu = i as usize;
    let mut info = MmdInstrumentInfo::default();
    let mut instr = InstrHdr::default();
    let pos = hio_tell(f) as i32;

    /* Sanity check */
    if smp_idx >= m.mod_.smp {
        return -1;
    }

    read_synth_header(synth, f);

    /* Sanity check */
    if synth.voltbllen > 128 || synth.wftbllen > 128 || synth.wforms < 1 || synth.wforms > 64 {
        return -1;
    }

    for j in 0..synth.wforms as usize {
        synth.wf[j] = hio_read32b(f);
    }

    if hio_error(f) != 0 {
        return -1;
    }

    hio_seek(f, wf_pos(pos, synth.wf[0]), SEEK_SET);
    instr.length = hio_read32b(f);
    instr.type_ = hio_read16b(f) as i16;

    /* Hybrids using IFFOCT/ext samples as their sample don't seem to
     * exist. If one is found, this should be fixed. OctaMED SS 1.03
     * converts 16-bit samples to 8-bit when changed to hybrid. */
    if instr.type_ != 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    if libxmp_med_new_instrument_extras(&mut mod_.xxi[iu]) != 0 {
        return -1;
    }

    mod_.xxi[iu].nsm = synth.wforms as i32;
    if libxmp_alloc_subinstrument(mod_, i, synth.wforms as i32) < 0 {
        return -1;
    }

    let xxi = &mut mod_.xxi[iu];
    if let Some(ie) = med_instrument_extras_mut(xxi) {
        ie.vts = synth.volspeed as i32;
        ie.wts = synth.wfspeed as i32;
        ie.vtlen = synth.voltbllen as i32;
        ie.wtlen = synth.wftbllen as i32;
    }

    mmd_load_instrument_common(&mut info, Some(&instr), expdata, exp_smp, sample, ver);
    mmd_set_default_pitch_note(xxi, exp_smp, ver);
    let fin = ((exp_smp.finetune as u8) << 4) as i8 as i32;
    let sub = &mut xxi.sub[0];

    sub.pan = XMP_INST_NO_DEFAULT_PAN;
    sub.vol = if info.enable { sample.svol as i32 } else { 0 };
    sub.xpo = info.sampletrans;
    sub.sid = smp_idx;
    sub.fin = fin;

    let xxs = &mut mod_.xxs[smp_idx as usize];

    xxs.len = info.length as i32;
    xxs.lps = info.rep as i32;
    xxs.lpe = info.rep.wrapping_add(info.replen) as i32;
    xxs.flg = info.flg;

    let smpctl = m.smpctl;
    if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[smp_idx as usize], None) < 0 {
        return -1;
    }

    smp_idx += 1;

    for j in 1..synth.wforms as usize {
        let mod_ = &mut m.mod_;

        /* Sanity check */
        if j as i32 >= mod_.xxi[iu].nsm || smp_idx >= mod_.smp {
            return -1;
        }

        let sub = &mut mod_.xxi[iu].sub[j];
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.vol = if info.enable { 64 } else { 0 };
        sub.xpo = info.synthtrans;
        sub.sid = smp_idx;
        sub.fin = fin;

        hio_seek(f, wf_pos(pos, synth.wf[j]), SEEK_SET);

        let xxs = &mut mod_.xxs[smp_idx as usize];
        xxs.len = hio_read16b(f) as i32 * 2;
        xxs.lps = 0;
        xxs.lpe = xxs.len;
        xxs.flg = XMP_SAMPLE_LOOP;

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[smp_idx as usize], None) < 0 {
            return -1;
        }

        smp_idx += 1;
    }
    0
}

/// Translation of `mmd_load_synth_instrument()`.
#[allow(clippy::too_many_arguments)]
fn mmd_load_synth_instrument(
    f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
    mut smp_idx: i32,
    synth: &mut SynthInstr,
    expdata: &Mmd0Exp,
    exp_smp: &InstrExt,
    sample: &Mmd0Sample,
    ver: i32,
) -> i32 {
    let iu = i as usize;
    let mut info = MmdInstrumentInfo::default();
    let pos = hio_tell(f) as i32;

    mmd_load_instrument_common(&mut info, None, expdata, exp_smp, sample, ver);
    mmd_set_default_pitch_note(&mut m.mod_.xxi[iu], exp_smp, ver);

    read_synth_header(synth, f);

    if synth.wforms == 0xffff {
        m.mod_.xxi[iu].nsm = 0;
        return 1;
    }
    if synth.voltbllen > 128 || synth.wftbllen > 128 || synth.wforms > 64 {
        return -1;
    }

    for j in 0..synth.wforms as usize {
        synth.wf[j] = hio_read32b(f);
    }

    if hio_error(f) != 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    if libxmp_med_new_instrument_extras(&mut mod_.xxi[iu]) != 0 {
        return -1;
    }

    mod_.xxi[iu].nsm = synth.wforms as i32;
    if libxmp_alloc_subinstrument(mod_, i, synth.wforms as i32) < 0 {
        return -1;
    }

    if let Some(ie) = med_instrument_extras_mut(&mut mod_.xxi[iu]) {
        ie.vts = synth.volspeed as i32;
        ie.wts = synth.wfspeed as i32;
        ie.vtlen = synth.voltbllen as i32;
        ie.wtlen = synth.wftbllen as i32;
    }

    let fin = ((exp_smp.finetune as u8) << 4) as i8 as i32;
    for j in 0..synth.wforms as usize {
        let mod_ = &mut m.mod_;

        /* Sanity check */
        if j as i32 >= mod_.xxi[iu].nsm || smp_idx >= mod_.smp {
            return -1;
        }

        let sub = &mut mod_.xxi[iu].sub[j];
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.vol = if info.enable { 64 } else { 0 };
        sub.xpo = info.synthtrans;
        sub.sid = smp_idx;
        sub.fin = fin;

        hio_seek(f, wf_pos(pos, synth.wf[j]), SEEK_SET);

        let xxs = &mut mod_.xxs[smp_idx as usize];
        xxs.len = hio_read16b(f) as i32 * 2;
        xxs.lps = 0;
        xxs.lpe = xxs.len;
        xxs.flg = XMP_SAMPLE_LOOP;

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[smp_idx as usize], None) < 0 {
            return -1;
        }

        smp_idx += 1;
    }

    0
}

/// Translation of `mmd_load_sampled_instrument()`.
#[allow(clippy::too_many_arguments)]
fn mmd_load_sampled_instrument(
    f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
    smp_idx: i32,
    instr: &InstrHdr,
    expdata: &Mmd0Exp,
    exp_smp: &InstrExt,
    sample: &Mmd0Sample,
    ver: i32,
) -> i32 {
    let iu = i as usize;
    let mut info = MmdInstrumentInfo::default();

    /* Sanity check */
    if smp_idx >= m.mod_.smp {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* hold & decay support */
    if libxmp_med_new_instrument_extras(&mut mod_.xxi[iu]) != 0 {
        return -1;
    }
    if let Some(ie) = med_instrument_extras_mut(&mut mod_.xxi[iu]) {
        ie.hold = exp_smp.hold as i32;
        ie.decay = exp_smp.decay as i32;
    }

    mod_.xxi[iu].nsm = 1;
    if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
        return -1;
    }

    mmd_load_instrument_common(&mut info, Some(instr), expdata, exp_smp, sample, ver);
    let xxi = &mut mod_.xxi[iu];
    mmd_set_default_pitch_note(xxi, exp_smp, ver);
    let sub = &mut xxi.sub[0];

    sub.vol = if info.enable { sample.svol as i32 } else { 0 };
    sub.pan = XMP_INST_NO_DEFAULT_PAN;
    sub.xpo = info.sampletrans;
    sub.sid = smp_idx;
    sub.fin = ((exp_smp.finetune as u8) << 4) as i8 as i32;

    let xxs = &mut mod_.xxs[smp_idx as usize];

    xxs.len = info.length as i32;
    xxs.lps = info.rep as i32;
    xxs.lpe = info.rep.wrapping_add(info.replen) as i32;
    xxs.flg = info.flg;

    /* Restrict sampled instruments to 3 octave range except for MMD3.
     * Checked in MMD0 with med.egypian/med.medieval from Lemmings 2
     * and MED.ParasolStars, MMD1 with med.Lemmings2
     */

    if ver < 3 {
        /* ExtSamples have two extra octaves. */
        let octaves = if (instr.type_ & 7) == 7 { 5 } else { 3 };
        for j in 0..9 {
            for k in 0..12 {
                let mut xpo = 0;

                if j < 1 {
                    xpo = 12 * (1 - j);
                } else if j > octaves {
                    xpo = -12 * (j - octaves);
                }

                xxi.map[(12 * j + k) as usize].xpo = xpo as i8;
            }
        }
    }

    let smpctl = m.smpctl;
    if libxmp_load_sample(
        smpctl,
        Some(f),
        SAMPLE_FLAG_BIGEND,
        &mut m.mod_.xxs[smp_idx as usize],
        None,
    ) < 0
    {
        return -1;
    }

    0
}

#[rustfmt::skip]
static IFFOCT_INSMAP: [[i8; 9]; 6] = [
    /* 2 */ [ 1, 1, 1, 0, 0, 0, 0, 0, 0 ],
    /* 3 */ [ 2, 2, 2, 2, 2, 2, 1, 1, 0 ],
    /* 4 */ [ 3, 3, 3, 2, 2, 2, 1, 1, 0 ],
    /* 5 */ [ 4, 4, 4, 3, 2, 2, 1, 1, 0 ],
    /* 6 */ [ 5, 5, 5, 5, 4, 3, 2, 1, 0 ],
    /* 7 */ [ 6, 6, 6, 6, 5, 4, 3, 2, 1 ],
];

#[rustfmt::skip]
static IFFOCT_XPOMAP: [[i8; 9]; 6] = [
    /* 2 */ [ 12, 12, 12,  0,  0,  0,  0,  0,  0 ],
    /* 3 */ [ 12, 12, 12, 12, 12, 12,  0,  0,-12 ],
    /* 4 */ [ 12, 12, 12,  0,  0,  0,-12,-12,-24 ],
    /* 5 */ [ 24, 24, 24, 12,  0,  0,-12,-24,-36 ],
    /* 6 */ [ 12, 12, 12, 12,  0,-12,-24,-36,-48 ],
    /* 7 */ [ 12, 12, 12, 12,  0,-12,-24,-36,-48 ],
];

/// Translation of `mmd_load_iffoct_instrument()`.
#[allow(clippy::too_many_arguments)]
fn mmd_load_iffoct_instrument(
    f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
    mut smp_idx: i32,
    instr: &InstrHdr,
    num_oct: i32,
    expdata: &Mmd0Exp,
    exp_smp: &InstrExt,
    sample: &Mmd0Sample,
    ver: i32,
) -> i32 {
    let iu = i as usize;
    let mut info = MmdInstrumentInfo::default();

    if !(2..=7).contains(&num_oct) {
        return -1;
    }

    /* Sanity check */
    if smp_idx + num_oct > m.mod_.smp {
        return -1;
    }

    /* Sanity check - ignore absurdly large IFFOCT instruments. */
    if (instr.length as i32) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* hold & decay support */
    if libxmp_med_new_instrument_extras(&mut mod_.xxi[iu]) != 0 {
        return -1;
    }

    if let Some(ie) = med_instrument_extras_mut(&mut mod_.xxi[iu]) {
        ie.hold = exp_smp.hold as i32;
        ie.decay = exp_smp.decay as i32;
    }

    mod_.xxi[iu].nsm = num_oct;
    if libxmp_alloc_subinstrument(mod_, i, num_oct) < 0 {
        return -1;
    }

    /* base octave size */
    let mut size = instr.length / ((1u32 << num_oct) - 1);
    mmd_load_instrument_common(&mut info, Some(instr), expdata, exp_smp, sample, ver);

    let fin = ((exp_smp.finetune as u8) << 4) as i8 as i32;
    for j in 0..num_oct as usize {
        let mod_ = &mut m.mod_;
        let sub = &mut mod_.xxi[iu].sub[j];

        sub.vol = if info.enable { sample.svol as i32 } else { 0 };
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.xpo = info.sampletrans - 12;
        sub.sid = smp_idx;
        sub.fin = fin;

        let xxs = &mut mod_.xxs[smp_idx as usize];

        xxs.len = size as i32;
        xxs.lps = info.rep as i32;
        xxs.lpe = info.rep.wrapping_add(info.replen) as i32;
        xxs.flg = info.flg;

        let smpctl = m.smpctl;
        if libxmp_load_sample(
            smpctl,
            Some(f),
            SAMPLE_FLAG_BIGEND,
            &mut m.mod_.xxs[smp_idx as usize],
            None,
        ) < 0
        {
            return -1;
        }

        smp_idx += 1;
        size <<= 1;
        info.rep <<= 1;
        info.replen <<= 1;
    }

    /* instrument mapping */

    let xxi = &mut m.mod_.xxi[iu];
    for j in 0..9 {
        for k in 0..12 {
            xxi.map[12 * j + k].ins = IFFOCT_INSMAP[num_oct as usize - 2][j] as u8;
            xxi.map[12 * j + k].xpo = IFFOCT_XPOMAP[num_oct as usize - 2][j];
        }
    }

    0
}

/* Number of octaves in IFFOCT samples */
pub(crate) const MMD_NUM_OCT: [i32; 6] = [5, 3, 2, 4, 6, 7];

/// Translation of `mmd_load_instrument()`: the next sample number, or -1.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mmd_load_instrument(
    f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
    mut smp_idx: i32,
    expdata: &Mmd0Exp,
    exp_smp: &InstrExt,
    sample: &Mmd0Sample,
    ver: i32,
) -> i32 {
    let mut instr = InstrHdr::default();
    let mut synth = SynthInstr::default();

    instr.length = hio_read32b(f);
    instr.type_ = hio_read16b(f) as i16;
    let sample_type = instr.type_ as i32 & !(S_16 | MD16 | STEREO);

    if instr.type_ == -2 {
        /* Hybrid */
        let ret =
            mmd_load_hybrid_instrument(f, m, i, smp_idx, &mut synth, expdata, exp_smp, sample, ver);

        if ret < 0 {
            return -1;
        }

        smp_idx += synth.wforms as i32;

        if mmd_alloc_tables(m, i, &synth) != 0 {
            return -1;
        }
    } else if instr.type_ == -1 {
        /* Synthetic */
        let ret =
            mmd_load_synth_instrument(f, m, i, smp_idx, &mut synth, expdata, exp_smp, sample, ver);

        if ret > 0 {
            return smp_idx;
        }

        if ret < 0 {
            return -1;
        }

        smp_idx += synth.wforms as i32;

        if mmd_alloc_tables(m, i, &synth) != 0 {
            return -1;
        }
    } else if instr.type_ >= 1 && instr.type_ <= 6 {
        /* IFFOCT */
        let oct = MMD_NUM_OCT[instr.type_ as usize - 1];

        let ret = mmd_load_iffoct_instrument(
            f, m, i, smp_idx, &instr, oct, expdata, exp_smp, sample, ver,
        );

        if ret < 0 {
            return -1;
        }

        smp_idx += oct;
    } else if sample_type == 0 || sample_type == 7 {
        /* Sample */
        let ret =
            mmd_load_sampled_instrument(f, m, i, smp_idx, &instr, expdata, exp_smp, sample, ver);

        if ret < 0 {
            return -1;
        }

        smp_idx += 1;
    } else {
        /* Invalid instrument type */
        return -1;
    }
    smp_idx
}

/* Load an external instrument (pre-MMD when the internal instruments flag is
 * not set). Returns 0 on successfully loading or if the instrument can't be
 * found. Returns -1 if an instrument is found but fails to load. */
/// Translation of `med_load_external_instrument()`.
pub(crate) fn med_load_external_instrument(
    _f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
) -> i32 {
    let iu = i as usize;
    let mut sp = LibxmpPath::default();
    let mut ins_name = [0u8; 32];

    let name = m.mod_.xxi[iu].name;
    if libxmp_copy_name_for_fopen(&mut ins_name, &name, 32) != 0 {
        return 0;
    }

    libxmp_path_init(&mut sp);
    if libxmp_find_instrument_file(m, &mut sp, &ins_name) != 0 {
        return 0;
    }

    let s = sp.path.as_deref().and_then(hio_open);
    libxmp_path_free(&mut sp);
    let Some(mut s) = s else {
        return 0;
    };

    m.mod_.xxs[iu].len = hio_size(&s) as i32;
    if m.mod_.xxs[iu].len == 0 {
        return 0;
    }
    m.mod_.xxi[iu].nsm = 1;

    let smpctl = m.smpctl;
    if libxmp_load_sample(smpctl, Some(&mut s), 0, &mut m.mod_.xxs[iu], None) < 0 {
        return -1;
    }
    0
}

/// Translation of `mmd_set_bpm()`.
pub(crate) fn mmd_set_bpm(
    m: &mut ModuleData,
    med_8ch: bool,
    deftempo: i32,
    bpm_on: bool,
    bpmlen: i32,
) {
    m.mod_.bpm = mmd_convert_tempo(deftempo, bpm_on, med_8ch);

    /* 8-channel mode completely overrides regular timing.
     * See mmd_convert_tempo for more info.
     */
    if med_8ch {
        m.time_factor = DEFAULT_TIME_FACTOR;
    } else if bpm_on {
        m.time_factor = DEFAULT_TIME_FACTOR * 4.0 / bpmlen as f64;
    }
}

/// Translation of `mmd_info_text()`.
pub(crate) fn mmd_info_text(f: &mut HioHandle<'_>, m: &mut ModuleData, _offset: i32) {
    /* Copy song info text */
    hio_read32b(f); /* skip next */
    hio_read16b(f); /* skip reserved */
    let type_ = hio_read16b(f);
    if type_ == 1 {
        /* 1 = ASCII */
        let len = hio_read32b(f) as i32;
        if len > 0 && (len as i64) < hio_size(f) {
            let Ok(mut comment) = try_vec(len as usize + 1, 0u8) else {
                return;
            };
            hio_read(&mut comment, 1, len as usize, f);
            comment[len as usize] = 0;
            m.comment = Some(comment);
        }
    }
}

/* Determine an approximate tracker version from an MMD module since, unlike
 * MED4, they don't store any useful tracker information. If expdata is not
 * present in the module, it should be passed as NULL.
 *
 * Other untested bits that may eventually be useful:
 * - OctaMED 3.00 changed the order the block array and sample array are
 *   written in. This might be more foolproof than the song name.
 * - InstrExt::finetune was "pad0" in OctaMED 2, should be non-zero only in 3+?
 * - i_ext_entrsz: MED 3.20 + OctaMED 2.00 (already tested by s_ext_entrsz)
 * - rgbtable: MED 3.20 + OctaMED 2.00 (already tested by s_ext_entrsz)
 *   transition.med has s_ext_entrsz=4 but no rgbtable, though?
 * - channel_split/n_info was added by OctaMED 2.00 (OctaMED only)? only saved
 *   if the module was edited in score mode?
 */
/// Translation of `mmd_tracker_version()`.
pub(crate) fn mmd_tracker_version(
    m: &mut ModuleData,
    mmdver: i32,
    mmdc: bool,
    med_8ch: bool,
    expdata: Option<&Mmd0Exp>,
) -> i32 {
    let mut soundstudio = 0;
    let medver;
    let mut s_ext_entrsz = 0;
    let mut songnamelen: u32 = 0;
    let mut mmdch = (b'0' as i32 + mmdver) as u8 as char;

    if let Some(expdata) = expdata {
        s_ext_entrsz = expdata.s_ext_entrsz as i32;
        songnamelen = expdata.songnamelen;
    }

    if s_ext_entrsz > 18 {
        /* s_ext_entrsz == 24 */
        medver = MED_VER_OCTAMED_SS_2;
        soundstudio = 2;
    } else if mmdver >= 3 {
        medver = MED_VER_OCTAMED_SS_1;
        soundstudio = 1;
    } else if s_ext_entrsz > 10 {
        /* s_ext_entrsz == 18 */
        medver = MED_VER_OCTAMED_SS_1;
        soundstudio = 1;
    } else if s_ext_entrsz > 8 {
        /* s_ext_entrsz == 10 */
        medver = MED_VER_OCTAMED_502;
    } else if s_ext_entrsz > 4 {
        /* s_ext_entrsz == 8 */
        medver = MED_VER_OCTAMED_500;
    } else if mmdver >= 2 {
        medver = MED_VER_OCTAMED_500;
    } else if songnamelen > 0 {
        /* Added in OctaMED 3.00 */
        medver = MED_VER_OCTAMED_300;
    } else if mmdver >= 1 {
        medver = MED_VER_OCTAMED_300;
    } else if s_ext_entrsz > 2 {
        /* s_ext_entrsz == 4 */
        medver = if med_8ch {
            MED_VER_OCTAMED_200
        } else {
            MED_VER_320
        };
    } else if expdata.is_some() {
        /* s_ext_entrsz == 2 */
        medver = if med_8ch {
            MED_VER_OCTAMED_100
        } else {
            MED_VER_300
        };
    } else {
        medver = MED_VER_210;
    }

    if mmdc {
        mmdch = 'C';
    }

    let s = if soundstudio == 2 {
        format!("MED Soundstudio 2.00 MMD{}", mmdch)
    } else if soundstudio == 1 {
        format!("OctaMED Soundstudio MMD{}", mmdch)
    } else if medver > MED_VER_320 {
        format!(
            "OctaMED {}.{:02x} MMD{}",
            medver >> 12,
            medver & 0xff,
            mmdch
        )
    } else {
        format!("MED {}.{:02x} MMD{}", medver >> 8, medver & 0xff, mmdch)
    };
    libxmp_set_type(m, s.as_bytes());
    medver
}
