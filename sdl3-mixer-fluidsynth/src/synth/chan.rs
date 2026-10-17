/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: translated into Rust from fluid_chan.c and
// fluid_chan.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE). The
// parts that call into the synth (fluid_channel_init(),
// fluid_channel_reset(), fluid_channel_cc_legato() and
// fluid_channel_cc_breath_note_on_off()) are synth methods in synth.rs.

use std::sync::Arc;

use crate::fluidsynth_h::*;
use crate::midi::midi::*;
use crate::sfloader::sfont::FluidPreset;
use crate::synth::gen::*;
use crate::synth::synth::{
    DRUM_INST_BANK, FLUID_BANK_STYLE_GM, FLUID_BANK_STYLE_GS, FLUID_BANK_STYLE_XG,
};
use crate::synth::tuning::FluidTuning;
use crate::utils::conv::{fluid_concave, fluid_convex};
use crate::utils::sys::{flog, FLUID_ERR};
use crate::FluidReal;

/* The mononophonic list is part of the legato detector for monophonic mode */
/* see fluid_synth_monopoly.c about a description of the legato detector device */
/* Size of the monophonic list
   - 1 is the minimum. it allows playing legato passage of any number
     of notes on noteon only.
   - Size above 1 allows playing legato on noteon but also on noteOff.
     This allows the  musician to play fast trills.
     This feature is particularly usful when the MIDI input device is a keyboard.
     Choosing a size of 10 is sufficient (because most musicians have only 10
     fingers when playing a monophonic instrument).
*/
pub(crate) const FLUID_CHANNEL_SIZE_MONOLIST: usize = 10;

/* Field shift amounts for sfont_bank_prog bit field integer */
const PROG_SHIFTVAL: i32 = 0;
const BANK_SHIFTVAL: i32 = 8;
const SFONT_SHIFTVAL: i32 = 22;

/* Field mask values for sfont_bank_prog bit field integer */
const PROG_MASKVAL: i32 = 0x000000FF; /* Bit 7 is used to indicate unset state */
const BANK_MASKVAL: i32 = 0x003FFF00;
const BANKLSB_MASKVAL: i32 = 0x00007F00;
const BANKMSB_MASKVAL: i32 = 0x003F8000;
const SFONT_MASKVAL: i32 = 0xFFC00000_u32 as i32;

/*
            The monophonic list
   +------------------------------------------------+
   |    +----+   +----+          +----+   +----+    |
   |    |note|   |note|          |note|   |note|    |
   +--->|vel |-->|vel |-->....-->|vel |-->|vel |----+
        +----+   +----+          +----+   +----+
         /|\                      /|\
          |                        |
       i_first                   i_last

 The monophonic list is a circular buffer of FLUID_CHANNEL_SIZE_MONOLIST elements.
 Each element is linked forward at initialisation time.
 - when a note is added at noteOn  (see fluid_channel_add_monolist()) each
   element is use in the forward direction and indexed by i_last variable.
 - when a note is removed at noteOff (see fluid_channel_remove_monolist()),
   the element concerned is fast unlinked and relinked after the i_last element.

 The most recent note added is indexed by i_last.
 The most ancient note added is the first note indexed by i_first. i_first is
 moving in the forward direction in a circular manner.

*/
/// Translation of `struct mononote`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Mononote {
    pub next: u8, /* next note */
    pub note: u8, /* note */
    pub vel: u8,  /* velocity */
}

/// One record of `fluid_channel_t::override_gen_default`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OverrideGenDefault {
    pub flags: u8,
    pub val: FluidReal,
}

/// Translation of `fluid_channel_t`.
///
/// Mutual exclusion notes (as of 1.1.2):
/// None - everything should have been synchronized by the synth.
#[derive(Clone)]
pub(crate) struct FluidChannel {
    pub channum: i32, /* MIDI channel number */

    /* Poly Mono variables see macro access description */
    pub mode: i32,     /* Poly Mono mode */
    pub mode_val: i32, /* number of channel in basic channel group */

    /* monophonic list - legato detector */
    pub i_first: u8,                                       /* First note index */
    pub i_last: u8,    /* most recent note index since the most recent add */
    pub prev_note: u8, /* previous note of the most recent add/remove */
    pub n_notes: u8,   /* actual number of notes in the list */
    pub monolist: [Mononote; FLUID_CHANNEL_SIZE_MONOLIST], /* monophonic list */

    pub key_mono_sustained: u8, /* previous sustained monophonic note */
    pub previous_cc_breath: u8, /* Previous Breath */
    pub legatomode: i32,        /* legato mode */
    pub portamentomode: i32,    /* portamento mode */
    /*- End of Poly/mono variables description */
    pub cc: [u8; 128],           /* MIDI controller values from [0;127] */
    pub key_pressure: [u8; 128], /* MIDI polyphonic key pressure from [0;127] */

    /* Drum channel flag, CHANNEL_TYPE_MELODIC, or CHANNEL_TYPE_DRUM. */
    pub channel_type: i32,
    pub interp_method: i32, /* Interpolation method (enum fluid_interp) */

    pub channel_pressure: u8, /* MIDI channel pressure from [0;127] */
    pub pitch_wheel_sensitivity: f32, /* Current pitch wheel sensitivity */
    pub pitch_bend: i16,      /* Current pitch bend value */
    /* Sostenuto order id gives the order of SostenutoOn event.
     * This value is useful to known when the sostenuto pedal is depressed
     * (before or after a key note). We need to compare SostenutoOrderId with voice id.
     */
    pub sostenuto_orderid: u32,

    pub tuning_bank: i32,                 /* Current tuning bank number */
    pub tuning_prog: i32,                 /* Current tuning program number */
    pub tuning: Option<Arc<FluidTuning>>, /* Micro tuning */

    pub preset: Option<FluidPreset>, /* Selected preset */
    pub sfont_bank_prog: i32, /* SoundFont ID (bit 21-31), bank (bit 7-20), program (bit 0-6) */

    /* NRPN system */
    pub nrpn_select: i32, /* Generator ID of SoundFont NRPN message */
    pub nrpn_active: i8,  /* 1 if data entry CCs are for NRPN, 0 if RPN */

    /* The values of the generators, set by NRPN messages, or by
     * fluid_synth_set_gen(), are cached in the channel so they can be
     * applied to future notes. They are copied to a voice's generators
     * in fluid_voice_init(), which calls fluid_gen_init().  */
    pub gen: [FluidReal; GEN_LAST],

    /* Same for AWE32 NRPNs, however they override the gen's default values */
    pub override_gen_default: [OverrideGenDefault; GEN_LAST],
}

impl std::fmt::Debug for FluidChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FluidChannel")
            .field("channum", &self.channum)
            .finish_non_exhaustive()
    }
}

/* Interface to poly/mono mode variables */
/* enum fluid_channel_mode_flags_internal */
pub(crate) const FLUID_CHANNEL_BASIC: i32 = 0x04; /* if flag set the corresponding midi channel is a basic channel */
pub(crate) const FLUID_CHANNEL_ENABLED: i32 = 0x08; /* if flag set the corresponding midi channel is enabled, else disabled, i.e. channel ignores any MIDI messages */

/*
  FLUID_CHANNEL_LEGATO_PLAYING bit of channel mode keeps trace of the legato /staccato
  state playing.
  FLUID_CHANNEL_LEGATO_PLAYING bit is updated on noteOn/noteOff mono by the legato detector:
  - On noteOn, before inserting a new note into the monolist.
  - On noteOff, after removing a note out of the monolist.

  - On noteOn, this state is used by fluid_synth_noteon_mono_LOCAL()
  to play the current  note legato or staccato.
  - On noteOff, this state is used by fluid_synth_noteoff_mono_LOCAL()
  to play the current noteOff legato with the most recent note.
*/
/* bit7, 1: means legato playing , 0: means staccato playing */
pub(crate) const FLUID_CHANNEL_LEGATO_PLAYING: i32 = 0x80;

/* Macros interface to poly/mono mode variables */
pub(crate) const MASK_BASICCHANINFOS: i32 =
    FLUID_CHANNEL_MODE_MASK | FLUID_CHANNEL_BASIC | FLUID_CHANNEL_ENABLED;

/* Macros interface to breath variables */
#[allow(dead_code)]
pub(crate) const FLUID_CHANNEL_BREATH_MASK: i32 =
    FLUID_CHANNEL_BREATH_POLY | FLUID_CHANNEL_BREATH_MONO | FLUID_CHANNEL_BREATH_SYNC;

/* Macros interface to monophonic list variables */
pub(crate) const INVALID_NOTE: u8 = 255;

/// `fluid_channel_is_valid_note()`: returns true when a note is a valid
/// note.
#[inline]
pub(crate) fn fluid_channel_is_valid_note<T: PartialEq + From<u8>>(n: T) -> bool {
    n != T::from(INVALID_NOTE)
}

impl FluidChannel {
    /// The fields `new_fluid_channel()` sets before `fluid_channel_init()`
    /// (which the synth runs, see `FluidSynthInner::fluid_channel_init()`),
    /// with the rest zeroed.
    pub(crate) fn new_uninit(num: i32) -> FluidChannel {
        FluidChannel {
            channum: num,
            mode: 0,
            mode_val: 0,
            i_first: 0,
            i_last: 0,
            prev_note: 0,
            n_notes: 0,
            monolist: [Mononote::default(); FLUID_CHANNEL_SIZE_MONOLIST],
            key_mono_sustained: 0,
            previous_cc_breath: 0,
            legatomode: 0,
            portamentomode: 0,
            cc: [0; 128],
            key_pressure: [0; 128],
            channel_type: 0,
            interp_method: 0,
            channel_pressure: 0,
            pitch_wheel_sensitivity: 0.0,
            pitch_bend: 0,
            sostenuto_orderid: 0,
            tuning_bank: 0,
            tuning_prog: 0,
            tuning: None,
            preset: None,
            sfont_bank_prog: 0,
            nrpn_select: 0,
            nrpn_active: 0,
            gen: [0.0; GEN_LAST],
            override_gen_default: [OverrideGenDefault::default(); GEN_LAST],
        }
    }

    /// The part of `fluid_channel_init()` before it looks up the preset:
    /// returns the bank and program to select.
    pub(crate) fn init_before_preset(&mut self) -> (i32, i32) {
        self.sostenuto_orderid = 0;
        /*--- Init poly/mono modes variables --------------------------------------*/
        self.mode = 0;
        self.mode_val = 0;

        /* monophonic list initialization */
        for i in 0..FLUID_CHANNEL_SIZE_MONOLIST {
            self.monolist[i].next = (i + 1) as u8;
        }

        self.monolist[FLUID_CHANNEL_SIZE_MONOLIST - 1].next = 0; /* ending element chained to the 1st */
        self.n_notes = 0;
        self.i_last = 0; /* clears the list */
        self.i_first = self.monolist[self.i_last as usize].next; /* first note index in the list */
        self.clear_prev_note(); /* Mark previous note invalid */
        /*---*/
        self.key_mono_sustained = INVALID_NOTE; /* No previous mono note sustained */
        self.legatomode = FLUID_CHANNEL_LEGATO_MODE_MULTI_RETRIGGER; /* Default mode */
        self.portamentomode = FLUID_CHANNEL_PORTAMENTO_MODE_EACH_NOTE; /* Default mode */
        /*--- End of poly/mono initialization --------------------------------------*/

        self.channel_type = if self.channum == 9 {
            CHANNEL_TYPE_DRUM
        } else {
            CHANNEL_TYPE_MELODIC
        };
        let prognum = 0;
        let banknum = if self.channel_type == CHANNEL_TYPE_DRUM {
            DRUM_INST_BANK
        } else {
            0
        };

        self.sfont_bank_prog =
            (0 << SFONT_SHIFTVAL) | (banknum << BANK_SHIFTVAL) | (prognum << PROG_SHIFTVAL);

        (banknum, prognum)
    }

    /// The part of `fluid_channel_init()` after it has set the preset.
    pub(crate) fn init_after_preset(&mut self) {
        self.interp_method = FLUID_INTERP_DEFAULT;
        self.tuning_bank = 0;
        self.tuning_prog = 0;
        self.nrpn_select = 0;
        self.nrpn_active = 0;

        if self.tuning.is_some() {
            self.tuning = None; /* (fluid_tuning_unref()) */
        }
    }

    /// Translation of `fluid_channel_init_ctrl()`.
    ///
    /// @param is_all_ctrl_off if nonzero, only resets some controllers, according to
    /// https://www.midi.org/techspecs/rp15.php
    pub(crate) fn init_ctrl(&mut self, is_all_ctrl_off: bool) {
        self.channel_pressure = 0;
        self.pitch_bend = 0x2000; /* Range is 0x4000, pitch bend wheel starts in centered position */

        for i in 0..GEN_LAST {
            self.gen[i] = 0.0;
            self.override_gen_default[i].flags = GEN_UNUSED;
            self.override_gen_default[i].val = 0.0;
        }

        if is_all_ctrl_off {
            for i in 0..ALL_SOUND_OFF {
                if (EFFECTS_DEPTH1..=EFFECTS_DEPTH5).contains(&i) {
                    continue;
                }

                if (SOUND_CTRL1..=SOUND_CTRL10).contains(&i) {
                    continue;
                }

                if i == BANK_SELECT_MSB
                    || i == BANK_SELECT_LSB
                    || i == VOLUME_MSB
                    || i == VOLUME_LSB
                    || i == PAN_MSB
                    || i == PAN_LSB
                    || i == BALANCE_MSB
                    || i == BALANCE_LSB
                {
                    continue;
                }

                self.set_cc(i, 0);
            }
        } else {
            for i in 0..128 {
                self.set_cc(i, 0);
            }

            self.previous_cc_breath = 0; /* Reset previous breath */
        }
        /* Unconditionally clear PTC receive (issue #1050) */
        self.clear_portamento();

        /* Reset polyphonic key pressure on all voices */
        for i in 0..128 {
            self.key_pressure[i] = 0;
        }

        /* Set RPN controllers to NULL state */
        self.set_cc(RPN_LSB, 127);
        self.set_cc(RPN_MSB, 127);

        /* Set NRPN controllers to NULL state */
        self.set_cc(NRPN_LSB, 127);
        self.set_cc(NRPN_MSB, 127);

        /* Expression (MSB & LSB) */
        self.set_cc(EXPRESSION_MSB, 127);
        self.set_cc(EXPRESSION_LSB, 127);

        if !is_all_ctrl_off {
            self.pitch_wheel_sensitivity = 2.0; /* two semi-tones */

            /* Just like panning, a value of 64 indicates no change for sound ctrls */
            for i in SOUND_CTRL1..=SOUND_CTRL10 {
                self.set_cc(i, 64);
            }

            /* Volume / initial attenuation (MSB & LSB) */
            self.set_cc(VOLUME_MSB, 100);
            self.set_cc(VOLUME_LSB, 0);

            /* Pan (MSB & LSB) */
            self.set_cc(PAN_MSB, 64);
            self.set_cc(PAN_LSB, 0);

            /* Balance (MSB & LSB) */
            self.set_cc(BALANCE_MSB, 64);
            self.set_cc(BALANCE_LSB, 0);

            /* Reverb */
            /* fluid_channel_set_cc (chan, EFFECTS_DEPTH1, 40); */
            /* Note: although XG standard specifies the default amount of reverb to
            be 40, most people preferred having it at zero.
            See https://lists.gnu.org/archive/html/fluid-dev/2009-07/msg00016.html */
        }
    }

    /// Translation of `fluid_channel_set_preset()`. Should only be called
    /// from synthesis context.
    pub(crate) fn set_preset(&mut self, preset: Option<FluidPreset>) -> i32 {
        if FluidPreset::opt_ptr_eq(self.preset.as_ref(), preset.as_ref()) {
            return FLUID_OK;
        }

        // (the soundfont reference counts and the preset notifications,
        // which only dynamic sample loading uses, are the Arcs)
        self.preset = preset;

        FLUID_OK
    }

    /// Translation of `fluid_channel_set_sfont_bank_prog()`: set SoundFont
    /// ID, MIDI bank and/or program.  Use -1 to use current value.
    pub(crate) fn set_sfont_bank_prog(&mut self, sfontnum: i32, banknum: i32, prognum: i32) {
        let mut newval = (if sfontnum != -1 {
            sfontnum.wrapping_shl(SFONT_SHIFTVAL as u32)
        } else {
            0
        }) | (if banknum != -1 {
            banknum.wrapping_shl(BANK_SHIFTVAL as u32)
        } else {
            0
        }) | (if prognum != -1 {
            prognum << PROG_SHIFTVAL
        } else {
            0
        });

        let oldmask = (if sfontnum != -1 { 0 } else { SFONT_MASKVAL })
            | (if banknum != -1 { 0 } else { BANK_MASKVAL })
            | (if prognum != -1 { 0 } else { PROG_MASKVAL });

        let oldval = self.sfont_bank_prog;
        newval = (newval & !oldmask) | (oldval & oldmask);
        self.sfont_bank_prog = newval;
    }

    /// Translation of `fluid_channel_set_bank_lsb()`: set bank LSB 7 bits.
    pub(crate) fn set_bank_lsb(&mut self, bank_select: i32, banklsb: i32) {
        let style = bank_select;

        if style == FLUID_BANK_STYLE_GM || style == FLUID_BANK_STYLE_GS {
            return; /* ignored */
        }

        let oldval = self.sfont_bank_prog;
        let newval = if style == FLUID_BANK_STYLE_XG {
            if self.channel_type == CHANNEL_TYPE_DRUM {
                return; // bankLSB is ignored for drum channels
            }
            (oldval & !BANK_MASKVAL) | (banklsb << BANK_SHIFTVAL)
        } else {
            /* style == FLUID_BANK_STYLE_MMA */
            (oldval & !BANKLSB_MASKVAL) | (banklsb << BANK_SHIFTVAL)
        };

        self.sfont_bank_prog = newval;
    }

    /// Translation of `fluid_channel_set_bank_msb()`: set bank MSB 7 bits.
    pub(crate) fn set_bank_msb(&mut self, bank_select: i32, mut bankmsb: i32) {
        let style = bank_select;
        let oldval = self.sfont_bank_prog;
        let newval;

        if style == FLUID_BANK_STYLE_XG {
            /* XG bank, do drum-channel auto-switch */
            /* The number "120" was based on several keyboards having drums at 120 - 127,
            reference: https://lists.nongnu.org/archive/html/fluid-dev/2011-02/msg00003.html */
            self.channel_type = if 120 == bankmsb || 126 == bankmsb || 127 == bankmsb {
                CHANNEL_TYPE_DRUM
            } else {
                CHANNEL_TYPE_MELODIC
            };
            if self.channel_type == CHANNEL_TYPE_MELODIC {
                // bankMSB is ignored for meldodic channels
                return;
            }

            // ...but for drum channels, hardcode SF2 drum bank 128. Ideally, we should use bankMSB as bank number.
            // But we'd need to ensure that it's not a melodic preset, see #1524.
            newval = (oldval & !BANK_MASKVAL) | (128 << BANK_SHIFTVAL);
        } else if style == FLUID_BANK_STYLE_GM {
            return; /* ignored */
        } else if style == FLUID_BANK_STYLE_GS {
            if self.channel_type == CHANNEL_TYPE_DRUM {
                bankmsb += DRUM_INST_BANK;
            }
            newval = (oldval & !BANK_MASKVAL) | (bankmsb << BANK_SHIFTVAL);
        } else {
            /* style == FLUID_BANK_STYLE_MMA */
            newval = (oldval & !BANKMSB_MASKVAL) | (bankmsb << (BANK_SHIFTVAL + 7));
        }

        self.sfont_bank_prog = newval;
    }

    /// Translation of `fluid_channel_get_sfont_bank_prog()`: get SoundFont
    /// ID, MIDI bank and program.
    pub(crate) fn get_sfont_bank_prog(&self) -> (i32, i32, i32) {
        let sfont_bank_prog = self.sfont_bank_prog;

        (
            ((sfont_bank_prog & SFONT_MASKVAL) as u32 >> SFONT_SHIFTVAL) as i32,
            (sfont_bank_prog & BANK_MASKVAL) >> BANK_SHIFTVAL,
            (sfont_bank_prog & PROG_MASKVAL) >> PROG_SHIFTVAL,
        )
    }

    /// Translation of `fluid_channel_get_key_pitch()`: compute the pitch
    /// for a key after applying Fluidsynth's tuning functionality and
    /// channel coarse/fine tunings.
    /// @param key MIDI note number (0-127)
    /// @return the pitch of the key
    pub(crate) fn get_key_pitch(&self, key: i32) -> FluidReal {
        if let Some(tuning) = &self.tuning {
            tuning.get_pitch(key)
                + 100.0f32 as FluidReal * self.get_gen(GEN_COARSETUNE)
                + self.get_gen(GEN_FINETUNE)
        } else {
            (key as f32 * 100.0f32) as FluidReal
        }
    }

    /// Translation of `fluid_channel_update_legato_staccato_state()`:
    /// updates legato/ staccato playing state.
    /// The function is called:
    /// - on noteon before adding a note into the monolist.
    /// - on noteoff after removing a note out of the monolist.
    fn update_legato_staccato_state(&mut self) {
        /* Updates legato/ staccato playing state */
        if self.n_notes != 0 {
            self.mode |= FLUID_CHANNEL_LEGATO_PLAYING; /* Legato state */
        } else {
            self.mode &= !FLUID_CHANNEL_LEGATO_PLAYING; /* Staccato state */
        }
    }

    /// Translation of `fluid_channel_add_monolist()`: adds a note into the
    /// monophonic list. The function is part of the legato detector.
    /// fluid_channel_add_monolist() is intended to be called by
    /// fluid_synth_noteon_mono_LOCAL().
    ///
    /// When a note is added at noteOn each element is use in the forward direction
    /// and indexed by i_last variable.
    ///
    /// @param key MIDI note number (0-127).
    /// @param vel MIDI velocity (0-127, 0=noteoff).
    /// @param onenote. When 1 the function adds the note but the monophonic list
    ///                 keeps only one note (used on noteOn poly).
    /// Note: i_last index keeps a trace of the most recent note added.
    ///       prev_note keeps a trace of the note prior i_last note.
    ///       FLUID_CHANNEL_LEGATO_PLAYING bit keeps trace of legato/staccato playing state.
    ///
    /// More information in FluidPolyMono-0004.pdf chapter 4 (Appendices).
    pub(crate) fn add_monolist(&mut self, key: u8, vel: u8, onenote: u8) {
        let mut i_last = self.i_last;
        /* Updates legato/ staccato playing state */
        self.update_legato_staccato_state();

        if self.n_notes != 0 {
            /* keeps trace of the note prior last note */
            self.prev_note = self.monolist[i_last as usize].note;
        }

        /* moves i_last forward before writing new note */
        i_last = self.monolist[i_last as usize].next;
        self.i_last = i_last; /* now ilast indexes the last note */
        self.monolist[i_last as usize].note = key; /* we save note and velocity */
        self.monolist[i_last as usize].vel = vel;

        if onenote != 0 {
            /* clears monolist before one note addition */
            self.i_first = i_last;
            self.n_notes = 0;
        }

        if (self.n_notes as usize) < FLUID_CHANNEL_SIZE_MONOLIST {
            self.n_notes += 1; /* updates n_notes */
        } else {
            /* The end of buffer is reach. So circular motion for i_first */
            /* i_first index is moved forward */
            self.i_first = self.monolist[i_last as usize].next;
        }
    }

    /// Translation of `fluid_channel_search_monolist()`: searching a note
    /// in the monophonic list. The function is part of the legato
    /// detector. fluid_channel_search_monolist() is intended to be called
    /// by fluid_synth_noteoff_mono_LOCAL().
    ///
    /// The search starts from the first note in the list indexed by i_first
    ///
    /// @param key MIDI note number (0-127) to search.
    /// @param i_prev pointer on returned index of the note prior the note to search.
    /// @return index of the note if find, FLUID_FAILED otherwise.
    pub(crate) fn search_monolist(&self, key: u8, i_prev: &mut i32) -> i32 {
        let mut n = self.n_notes as i16; /* number of notes in monophonic list */
        let mut i = self.i_first as i16; /* searching starts from i_first included */

        let mut j: i16 = 0;
        while j < n {
            if self.monolist[i as usize].note == key {
                if i == self.i_first as i16 {
                    /* tracking index of the previous note (i_prev) */
                    j = self.i_last as i16;
                    while (n as usize) < FLUID_CHANNEL_SIZE_MONOLIST {
                        j = self.monolist[j as usize].next as i16;
                        n += 1;
                    }

                    *i_prev = j as i32; /* returns index of the previous note */
                }

                return i as i32; /* returns index of the note to search */
            }

            *i_prev = i as i32; /* tracking index of the previous note (i_prev) */
            i = self.monolist[i as usize].next as i16; /* next element */
            j += 1;
        }

        FLUID_FAILED /* not found */
    }

    /// Translation of `fluid_channel_remove_monolist()`: removes a note
    /// from the monophonic list. The function is part of the legato
    /// detector. fluid_channel_remove_monolist() is intended to be called
    /// by fluid_synth_noteoff_mono_LOCAL().
    ///
    /// When a note is removed at noteOff the element concerned is fast unlinked
    /// and relinked after the i_last element.
    ///
    /// @param
    ///   i, index of the note to remove. If i is invalid or the list is
    ///      empty, the function do nothing and returns FLUID_FAILED.
    /// @param
    ///   On input, i_prev is a pointer on index of the note previous i.
    ///   On output i_prev is a pointer on index of the note previous i if i is the last note
    ///   in the list,FLUID_FAILED otherwise. When the returned index is valid it means
    ///   a legato detection on noteoff.
    ///
    /// Note: the following variables in Channel keeps trace of the situation.
    ///       - i_last index keeps a trace of the most recent note played even if
    ///       the list is empty.
    ///       - prev_note keeps a trace of the note removed if it is i_last.
    ///       - FLUID_CHANNEL_LEGATO_PLAYING bit keeps a trace of legato/staccato playing state.
    ///
    /// More information in FluidPolyMono-0004.pdf chapter 4 (Appendices).
    pub(crate) fn remove_monolist(&mut self, i: i32, i_prev: &mut i32) {
        let i_last = self.i_last;

        /* checks if index is valid */
        if i < 0 || i >= FLUID_CHANNEL_SIZE_MONOLIST as i32 || self.n_notes == 0 {
            *i_prev = FLUID_FAILED;
            if i < 0 || i >= FLUID_CHANNEL_SIZE_MONOLIST as i32 {
                // FIXME (upstream): the function goes on with an invalid
                // index, indexing the list out of bounds; its callers only
                // pass indices fluid_channel_search_monolist() found, but
                // this stops here all the same.
                return;
            }
        }

        /* The element is about to be removed and inserted between i_last and next */
        /* Note: when i is equal to i_last or equal to i_first, removing/inserting
        isn't necessary */
        if i == i_last as i32 {
            /* Removing/Inserting isn't necessary */
            /* keeps trace of the note prior last note */
            self.prev_note = self.monolist[i_last as usize].note;
            /* moves i_last backward to the previous  */
            // (kept inside the list: FIXME (upstream), see above)
            self.i_last = ((*i_prev as u32 as usize) % FLUID_CHANNEL_SIZE_MONOLIST) as u8;
        /* i_last index is moved backward */
        } else {
            /* i is before i_last */
            if i == self.i_first as i32 {
                /* Removing/inserting isn't necessary */
                /* i_first index is moved forward to the next element*/
                self.i_first = self.monolist[i as usize].next;
            } else {
                /* i is between i_first and i_last */
                /* Unlinks element i and inserts after i_last */
                // (*i_prev is an index fluid_channel_search_monolist() found
                // here, or FLUID_FAILED after an empty list, out of bounds
                // upstream: FIXME (upstream))
                let prev = (*i_prev as u32 as usize) % FLUID_CHANNEL_SIZE_MONOLIST;
                self.monolist[prev].next = self.monolist[i as usize].next; /* unlinks i */
                /*inserts i after i_last */
                self.monolist[i as usize].next = self.monolist[i_last as usize].next;
                self.monolist[i_last as usize].next = i as u8;
            }

            *i_prev = FLUID_FAILED;
        }

        self.n_notes = self.n_notes.wrapping_sub(1); /* updates the number of note in the list */
        /* Updates legato/ staccato playing state */
        self.update_legato_staccato_state();
    }

    /// Translation of `fluid_channel_clear_monolist()`: on noteOff on a
    /// polyphonic channel,the monophonic list is fully flushed.
    ///
    /// Note: i_last index keeps a trace of the most recent note played even if
    ///       the list is empty.
    ///       prev_note keeps a trace of the note i_last .
    ///       FLUID_CHANNEL_LEGATO_PLAYING bit keeps a trace of legato/staccato playing.
    pub(crate) fn clear_monolist(&mut self) {
        /* keeps trace off the most recent note played */
        self.prev_note = self.monolist[self.i_last as usize].note;

        /* flushes the monolist */
        self.i_first = self.monolist[self.i_last as usize].next;
        self.n_notes = 0;
        /* Update legato/ sataccato playing state */
        self.mode &= !FLUID_CHANNEL_LEGATO_PLAYING; /* Staccato state */
    }

    /// Translation of `fluid_channel_set_onenote_monolist()`: on noteOn on
    /// a polyphonic channel,adds the note into the monophonic list keeping
    /// only this note.
    /// @param
    ///   key, vel, note and velocity added in the monolist
    /// Note: i_last index keeps a trace of the most recent note inserted.
    ///       prev_note keeps a trace of the note prior i_last note.
    ///       FLUID_CHANNEL_LEGATO_PLAYING bit keeps trace of legato/staccato playing.
    pub(crate) fn set_onenote_monolist(&mut self, key: u8, vel: u8) {
        self.add_monolist(key, vel, 1);
    }

    /// Translation of `fluid_channel_invalid_prev_note_staccato()`: the
    /// function changes the state (Valid/Invalid) of the previous note
    /// played in a staccato manner (fluid_channel_prev_note()).
    /// When potamento mode 'each note' or 'staccato only' is selected, on next
    /// noteOn a portamento will be started from the most recent note played
    /// staccato.
    /// It will be possible that it isn't appropriate. To give the musician the
    /// possibility to choose a portamento from this note , prev_note will be forced
    /// to invalid state on noteOff if portamento pedal is Off.
    ///
    /// The function is intended to be called when the following event occurs:
    /// - On noteOff (in poly or mono mode), to mark prev_note invalid.
    /// - On Portamento Off(in poly or mono mode), to mark prev_note invalid.
    pub(crate) fn invalid_prev_note_staccato(&mut self) {
        /* checks if the playing is staccato */
        if self.mode & FLUID_CHANNEL_LEGATO_PLAYING == 0 {
            /* checks if portamento pedal is off */
            if !self.portamento() {
                /* forces prev_note invalid */
                self.clear_prev_note();
            }
        }

        /* else prev_note still remains valid for next fromkey portamento */
    }

    /// Translation of `fluid_channel_get_override_gen_default()`.
    pub(crate) fn get_override_gen_default(&self, gen: usize, val: &mut FluidReal) -> bool {
        if self.override_gen_default[gen].flags != GEN_UNUSED {
            *val = self.override_gen_default[gen].val;
            return true;
        }

        false
    }

    /// Translation of `fluid_channel_set_override_gen_default()`.
    pub(crate) fn set_override_gen_default(&mut self, gen: usize, val: FluidReal) {
        self.override_gen_default[gen].flags = GEN_SET;
        self.override_gen_default[gen].val = val;
    }

    /// Translation of `fluid_channel_portamentotime_with_mode()`:
    /// calculate portamento time in milliseconds considering the
    /// synthesizer's portamento time mode.
    pub(crate) fn portamentotime_with_mode(
        &self,
        mut time_mode: i32,
        lsb_seen: bool,
        fromkey: i32,
        tokey: i32,
    ) -> u32 {
        let msb = self.get_cc(PORTAMENTO_TIME_MSB);
        let lsb = self.get_cc(PORTAMENTO_TIME_LSB);
        let mut res: i32;
        let tmp: FluidReal;
        const MAX: i32 = 480 * 1000; /*ms*/

        /* Use 7-bit MSB initially, switch to 14-bit if LSB has been seen */
        if time_mode == FLUID_PORTAMENTO_TIME_MODE_AUTO {
            if lsb_seen {
                time_mode = FLUID_PORTAMENTO_TIME_MODE_LINEAR;
            } else {
                time_mode = FLUID_PORTAMENTO_TIME_MODE_XG_GS;
            }
        }

        match time_mode {
            FLUID_PORTAMENTO_TIME_MODE_XG_GS => {
                // Produce a curve similar to:
                /*
                    CC 5 value  Portamento time
                    ----------  ---------------
                        0            0.000 s
                        1            0.006 s
                        2            0.023 s
                        4            0.050 s
                        8            0.110 s
                        16           0.250 s
                        32           0.500 s
                        64           2.060 s
                        80           4.200 s
                        96           8.400 s
                        112         19.500 s
                        116         26.700 s
                        120         40.000 s
                        124         80.000 s
                        127        480.000 s
                */
                // Tests were performed by John Novak
                // https://github.com/dosbox-staging/dosbox-staging/pull/2705
                tmp = fluid_concave(msb as FluidReal);
                res = ((((MAX / 2) as f64 * 2.5) as FluidReal) * tmp * fluid_concave(128.0 * tmp)
                    + 400.0 * fluid_convex(msb as FluidReal * (1.0 / 4.0) as FluidReal))
                    as i32;
                res = if res < MAX { res } else { MAX };
                // Apply a similar scaling hack as SpessaSynth to fix Descent Game08, it's unclear why exactly
                // https://github.com/spessasus/spessasynth_core/blob/5a8730a80f8c0b74733ec193a968b36e2a0c0aee/src/synthesizer/audio_engine/engine_methods/portamento_time.ts#L84-87
                // https://github.com/FluidSynth/fluidsynth/pull/1656#issuecomment-3355759938
                res = ((res.wrapping_mul((tokey - fromkey).wrapping_abs()) as f32 / 36.0f32
                    + 0.5f32) as u32) as i32;
                res as u32
            }

            FLUID_PORTAMENTO_TIME_MODE_LINEAR => {
                /* Always use 14-bit MSB+LSB */
                (msb * 128 + lsb) as u32
            }

            _ => {
                flog!(
                    FLUID_ERR,
                    "THIS SHOULD NEVER HAPPEN! unknown portamento time mode {}",
                    time_mode
                );
                0
            }
        }
    }

    /* fluid_chan.h's macros */

    /// `fluid_channel_get_preset()`
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn get_preset(&self) -> Option<&FluidPreset> {
        self.preset.as_ref()
    }
    /// `fluid_channel_set_cc()`
    #[inline]
    pub(crate) fn set_cc(&mut self, num: i32, val: i32) {
        self.cc[num as usize] = val as u8;
    }
    /// `fluid_channel_get_cc()`
    #[inline]
    pub(crate) fn get_cc(&self, num: i32) -> i32 {
        self.cc[num as usize] as i32
    }
    /// `fluid_channel_get_key_pressure()`
    #[inline]
    pub(crate) fn get_key_pressure(&self, key: i32) -> i32 {
        self.key_pressure[key as usize] as i32
    }
    /// `fluid_channel_set_key_pressure()`
    #[inline]
    pub(crate) fn set_key_pressure(&mut self, key: i32, val: i32) {
        self.key_pressure[key as usize] = val as u8;
    }
    /// `fluid_channel_get_channel_pressure()`
    #[inline]
    pub(crate) fn get_channel_pressure(&self) -> i32 {
        self.channel_pressure as i32
    }
    /// `fluid_channel_set_channel_pressure()`
    #[inline]
    pub(crate) fn set_channel_pressure(&mut self, val: i32) {
        self.channel_pressure = val as u8;
    }
    /// `fluid_channel_get_pitch_bend()`
    #[inline]
    pub(crate) fn get_pitch_bend(&self) -> i32 {
        self.pitch_bend as i32
    }
    /// `fluid_channel_set_pitch_bend()`
    #[inline]
    pub(crate) fn set_pitch_bend(&mut self, val: i32) {
        self.pitch_bend = val as i16;
    }
    /// `fluid_channel_get_pitch_wheel_sensitivity()`
    #[inline]
    pub(crate) fn get_pitch_wheel_sensitivity(&self) -> f32 {
        self.pitch_wheel_sensitivity
    }
    /// `fluid_channel_set_pitch_wheel_sensitivity()`
    #[inline]
    pub(crate) fn set_pitch_wheel_sensitivity(&mut self, val: f32) {
        self.pitch_wheel_sensitivity = val;
    }
    /// `fluid_channel_get_num()`
    #[inline]
    pub(crate) fn get_num(&self) -> i32 {
        self.channum
    }
    /// `fluid_channel_set_interp_method()`
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn set_interp_method(&mut self, new_method: i32) {
        self.interp_method = new_method;
    }
    /// `fluid_channel_get_interp_method()`
    #[inline]
    pub(crate) fn get_interp_method(&self) -> i32 {
        self.interp_method
    }
    /// `fluid_channel_set_tuning()`
    #[inline]
    pub(crate) fn set_tuning(&mut self, t: Option<Arc<FluidTuning>>) {
        self.tuning = t;
    }
    /// `fluid_channel_has_tuning()`
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn has_tuning(&self) -> bool {
        self.tuning.is_some()
    }
    /// `fluid_channel_get_tuning()`
    #[inline]
    pub(crate) fn get_tuning(&self) -> Option<&Arc<FluidTuning>> {
        self.tuning.as_ref()
    }
    /// `fluid_channel_get_tuning_bank()`
    #[inline]
    pub(crate) fn get_tuning_bank(&self) -> i32 {
        self.tuning_bank
    }
    /// `fluid_channel_set_tuning_bank()`
    #[inline]
    pub(crate) fn set_tuning_bank(&mut self, bank: i32) {
        self.tuning_bank = bank;
    }
    /// `fluid_channel_get_tuning_prog()`
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn get_tuning_prog(&self) -> i32 {
        self.tuning_prog
    }
    /// `fluid_channel_set_tuning_prog()`
    #[inline]
    pub(crate) fn set_tuning_prog(&mut self, prog: i32) {
        self.tuning_prog = prog;
    }
    /// `fluid_channel_portamento()`
    #[inline]
    pub(crate) fn portamento(&self) -> bool {
        self.cc[PORTAMENTO_SWITCH as usize] >= 64
    }
    /// `fluid_channel_breath_msb()`
    #[inline]
    pub(crate) fn breath_msb(&self) -> bool {
        self.cc[BREATH_MSB as usize] > 0
    }
    /// `fluid_channel_clear_portamento()`
    #[inline]
    pub(crate) fn clear_portamento(&mut self) {
        self.cc[PORTAMENTO_CTRL as usize] = INVALID_NOTE;
    }
    /// `fluid_channel_legato()`
    #[inline]
    pub(crate) fn legato(&self) -> bool {
        self.cc[LEGATO_SWITCH as usize] >= 64
    }
    /// `fluid_channel_sustained()`
    #[inline]
    pub(crate) fn sustained(&self) -> bool {
        self.cc[SUSTAIN_SWITCH as usize] >= 64
    }
    /// `fluid_channel_sostenuto()`
    #[inline]
    pub(crate) fn sostenuto(&self) -> bool {
        self.cc[SOSTENUTO_SWITCH as usize] >= 64
    }
    /// `fluid_channel_set_gen()`
    #[inline]
    pub(crate) fn set_gen(&mut self, n: usize, v: FluidReal) {
        self.gen[n] = v;
    }
    /// `fluid_channel_get_gen()`
    #[inline]
    pub(crate) fn get_gen(&self, n: usize) -> FluidReal {
        self.gen[n]
    }
    /// `fluid_channel_set_basic_channel_info()`: set the basic channel
    /// infos for a MIDI basic channel.
    #[inline]
    pub(crate) fn set_basic_channel_info(&mut self, infos: i32) {
        self.mode = (self.mode & !MASK_BASICCHANINFOS) | (infos & MASK_BASICCHANINFOS);
    }
    /// `fluid_channel_reset_basic_channel_info()`: reset the basic channel
    /// infos for a MIDI basic channel.
    #[inline]
    pub(crate) fn reset_basic_channel_info(&mut self) {
        self.mode &= !MASK_BASICCHANINFOS;
    }
    /// `fluid_channel_set_breath_info()`: set the breath infos for a MIDI
    /// channel.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn set_breath_info(&mut self, breath_infos: i32) {
        self.mode =
            (self.mode & !FLUID_CHANNEL_BREATH_MASK) | (breath_infos & FLUID_CHANNEL_BREATH_MASK);
    }
    /// `fluid_channel_get_breath_info()`: get the breath infos for a MIDI
    /// channel.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn get_breath_info(&self) -> i32 {
        self.mode & FLUID_CHANNEL_BREATH_MASK
    }
    /// `fluid_channel_is_playing_mono()`: returns true when channel is
    /// mono or legato is on.
    #[inline]
    pub(crate) fn is_playing_mono(&self) -> bool {
        (self.mode & FLUID_CHANNEL_POLY_OFF) != 0 || self.legato()
    }
    /// `fluid_channel_clear_prev_note()`: marks prev_note as invalid.
    #[inline]
    pub(crate) fn clear_prev_note(&mut self) {
        self.prev_note = INVALID_NOTE;
    }
    /// `fluid_channel_last_note()`: returns the most recent note from
    /// i_last entry of the monophonic list.
    #[inline]
    pub(crate) fn last_note(&self) -> u8 {
        self.monolist[self.i_last as usize].note
    }
    /// `fluid_channel_last_vel()`: returns the most recent velocity from
    /// i_last entry of the monophonic list.
    #[inline]
    pub(crate) fn last_vel(&self) -> u8 {
        self.monolist[self.i_last as usize].vel
    }
    /*
      prev_note is used to determine fromkey_portamento as well as
      fromkey_legato (see fluid_synth_get_fromkey_portamento_legato()).

      prev_note is updated on noteOn/noteOff mono by the legato detector as this:
      - On noteOn mono, before adding a new note into the monolist,the most
        recent  note in the list (i.e at i_last position) is kept in prev_note.
      - Similarly, on  noteOff mono , before removing a note out of the monolist,
        the most recent note (i.e those at i_last position) is kept in prev_note.
    */
    /// `fluid_channel_prev_note()`
    #[inline]
    pub(crate) fn prev_note(&self) -> u8 {
        self.prev_note
    }
}

/* The channel functions that call into the synth */
impl crate::synth::synth::FluidSynthInner {
    /// Translation of `fluid_channel_init()`.
    pub(crate) fn fluid_channel_init(&mut self, chan: usize) {
        let (banknum, prognum) = self.channel[chan].init_before_preset();

        let newpreset = self.fluid_synth_find_preset(banknum, prognum);
        self.channel[chan].set_preset(newpreset);

        self.channel[chan].init_after_preset();
    }

    /// Translation of `fluid_channel_reset()`.
    pub(crate) fn fluid_channel_reset(&mut self, chan: usize) {
        self.fluid_channel_init(chan);
        self.channel[chan].init_ctrl(false);
    }

    /// Translation of `fluid_channel_cc_legato()`: the function handles
    /// poly/mono commutation on legato pedal On/Off.
    /// @param value, value of the CC legato.
    pub(crate) fn fluid_channel_cc_legato(&mut self, chan: usize, value: i32) {
        let channel = &mut self.channel[chan];
        /* Special handling of the monophonic list  */
        if (channel.mode & FLUID_CHANNEL_POLY_OFF) == 0 && channel.n_notes != 0 {
            /* The monophonic list have notes */
            if value < 64 {
                /* legato is released */
                /* returns from monophonic to polyphonic with notes in the monophonic list */

                /* The monophonic list is flushed keeping last note only
                Note: i_last index keeps a trace of the most recent note played.
                prev_note keeps a trace of the note i_last.
                FLUID_CHANNEL_LEGATO_PLAYING bit keeps trace of legato/staccato playing.
                */
                channel.i_first = channel.i_last;
                channel.n_notes = 1;
            } else {
                /* legato is depressed */
                /* Inters in monophonic from polyphonic with note in monophonic list */
                /* Stops the running note to remain coherent with Breath Sync mode */
                if (channel.mode & FLUID_CHANNEL_BREATH_SYNC) != 0 && !channel.breath_msb() {
                    let channum = channel.channum;
                    let last = channel.last_note() as i32;
                    self.fluid_synth_noteoff_monopoly(channum, last, true);
                }
            }
        }
    }

    /// Translation of `fluid_channel_cc_breath_note_on_off()`: the function
    /// handles CC Breath On/Off detection. When a channel is in Breath Sync
    /// mode and in monophonic playing, the breath controller allows to
    /// trigger noteon/noteoff note when the musician starts to breath
    /// (noteon) and stops to breath (noteoff).
    /// @param value, value of the CC Breath..
    pub(crate) fn fluid_channel_cc_breath_note_on_off(&mut self, chan: usize, value: i32) {
        let channel = &self.channel[chan];
        if (channel.mode & FLUID_CHANNEL_BREATH_SYNC) != 0
            && channel.is_playing_mono()
            && channel.n_notes != 0
        {
            let channum = channel.channum;
            let last_note = channel.last_note() as i32;
            let last_vel = channel.last_vel() as i32;
            /* The monophonic list isn't empty */
            if (value > 0) && (channel.previous_cc_breath == 0) {
                /* CC Breath On detection */
                self.fluid_synth_noteon_mono_staccato(channum, last_note, last_vel);
            } else if (value == 0) && (channel.previous_cc_breath > 0) {
                /* CC Breath Off detection */
                self.fluid_synth_noteoff_monopoly(channum, last_note, true);
            }
        }

        self.channel[chan].previous_cc_breath = value as u8;
    }
}
