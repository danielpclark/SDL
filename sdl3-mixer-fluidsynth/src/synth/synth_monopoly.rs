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
// Modified 2026-10-17: translated into Rust from fluid_synth_monopoly.c in
// SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

use std::sync::atomic::Ordering;

use crate::fluidsynth_h::*;
use crate::midi::midi::PORTAMENTO_CTRL;
use crate::sfloader::defsfont::fluid_zone_inside_range;
use crate::synth::chan::*;
use crate::synth::synth::FluidSynthInner;
use crate::synth::voice::*;
use crate::utils::sys::{flog, FLUID_INFO, FLUID_WARN};

/******************************************************************************
  The legato detector is composed as this,
  variables:
  - monolist: monophonic list variable.
  - prev_note: to store the most recent note before adding on noteon or before
               removing on noteoff.
  - FLUID_CHANNEL_LEGATO_PLAYING: legato/staccato state bit that informs on
               legato or staccato playing.
  functions:
  - fluid_channel_add_monolist(), for inserting a new note.
  - fluid_channel_search_monolist(), for searching the position of a note
    into the list.
  - fluid_channel_remove_monolist(), for removing a note from the list.

            The monophonic list
   +------------------------------------------------+
   |    +----+   +----+          +----+   +----+    |
   |    |note|   |note|          |note|   |note|    |
   +--->|vel |-->|vel |-->....-->|vel |-->|vel |----+
        +----+   +----+          +----+   +----+
         /|\                      /|\
          |                        |
       i_first                   i_last

  The list allows an easy automatic detection of a legato passage when it is
  played on a MIDI keyboard input device.
  It is useful also when the input device is an ewi (electronic wind instrument)
  or evi (electronic valve instrument) and these instruments are unable to send
  MIDI CC legato on/off.

  The list memorizes the notes in playing order.
  - (a) On noteOn n2, if a previous note n1 exists, there is a legato
     detection with n1 (with or without portamento from n1 to n2 See note below).
  - (b) On noteOff of the running note n2, if a previous note n1 exists,
     there is a legato detection from n2 to n1, allowing fast trills playing
     (with or without portamento from n2 to n1. See note below).

  Notes in the list are inserted to the end of the list that works like a
  circular buffer.The features are:

  1) It is always possible to play an infinite legato passage in
     direct order (n1_On,n2_On,n3_On,....).

  2) Playing legato in the reverse order (n10_Off, n9_Off,,...) helps in
     fast trills playing as the list memorizes 10 most recent notes.

  3) Playing an infinite lagato passage in ascendant or descendant order,
     without playing trills is always possible using the usual way like this:
      First we begin with an ascendant passage,
      n1On, (n2On,n1Off), (n3On,n2Off) , (n4On,n3Off), then
      we continue with a descendant passage
      (n3On,n4off), (n2On,n3off), (n1On,n2off), n1Off...and so on

 Each MIDI channel have a legato detector.

 Note:
  Portamento is a feature independent of the legato detector. So
  portamento isn't part of the lagato detector. However portamento
  (when enabled) is triggered at noteOn (like legato). Like in legato
  situation it is usual to have a portamento from a note 'fromkey' to another
  note 'tokey'. Portamento fromkey note choice is determined at noteOn by
  fluid_synth_get_fromkey_portamento_legato() (see below).

  More information in FluidPolyMono-0004.pdf chapter 4 (Appendices).
******************************************************************************/

/*****************************************************************************
 Portamento related functions in Poly or Mono mode
******************************************************************************/

impl FluidSynthInner {
    /// Translation of `fluid_synth_get_fromkey_portamento_legato()`.
    ///
    /// fluid_synth_get_fromkey_portamento_legato returns two information:
    ///  - fromkey note for portamento.
    ///  - fromkey note for legato.
    ///
    /// ```text
    ///                                                 +-----> fromkey_portamento
    ///                                           ______|________
    ///                portamento modes >------->|               |
    ///                                          |  get_fromkey  |
    ///  Porta.on/off >------------------------->|_______________|
    ///  (PTC)                                          |
    ///                                                 +-----> fromkey_legato
    /// ```
    ///
    /// The functions is intended to be call on noteOn mono
    /// see fluid_synth_noteon_mono_staccato(), fluid_synth_noteon_monopoly_legato()
    /// -------
    /// 1)The function determines if a portamento must occur on next noteOn.
    /// The value returned is 'fromkey portamento' which is the pitchstart key
    /// of a portamento, as function of PTC or (default_fromkey, prev_note) both
    /// if Portamento On. By order of precedence the result is:
    ///  1.1) PTC have precedence over Portamento On.
    ///       If CC PTC has been received, its value supersedes and any
    ///       portamento pedal On, default_fromkey,prev_note or portamento mode.
    ///  1.2) Otherwise ,when Portamento On the function takes the following value:
    ///       - default_fromkey if valid
    ///       - otherwise prev_note(prev_note is the note prior the most recent
    ///       note played).
    ///       Then portamento mode is applied to validate the value chosen.
    ///       Where portamento mode is:
    ///       - each note, a portamento occurs on each note.
    ///       - legato only, portamento only on notes played legato.
    ///       - staccato only, portamento only on notes played staccato.
    ///  1.3) Otherwise, portamento is off,INVALID_NOTE is returned (portamento is disabled).
    /// ------
    /// 2)The function determines if a legato playing must occur on next noteOn.
    /// 'fromkey legato note' is returned as a function of default_fromkey, PTC,
    /// current mono/poly mode,actual 'staccato/legato' playing state and prev_note.
    /// By order of precedence the result is:
    ///  2.1) If valid, default_fromkey have precedence over any others values.
    ///  2.2) Otherwise if CC PTC has been received its value is returned.
    ///  2.3) Otherwise fromkey legato is determined from the mono/poly mode,
    ///       the actual 'staccato/legato' playing state (FLUID_CHANNEL_LEGATO_PLAYING) and prev_note
    ///       as this:
    ///       - in (poly/Mono) staccato , INVALID_NOTE is returned.
    ///       - in poly legato , actually we don't want playing legato. So
    ///         INVALID_NOTE is returned.
    ///       - in mono legato , prev_note is returned.
    ///
    /// On input
    /// @param chan  fluid_channel_t.
    /// @param defaultFromkey, the default 'fromkey portamento' note or 'fromkey legato'
    ///       note (see description above).
    ///
    /// @return
    ///  1)'fromkey portamento' is returned in fluid_synth_t.fromkey_portamento.
    ///  If valid,it means that portamento is enabled .
    ///
    ///  2)The 'fromkey legato' note is returned.
    ///
    /// Notes about usage:
    /// The function is intended to be called when the following event occurs:
    /// - On noteOn (Poly or Mono) after insertion in the monophonic list.
    /// - On noteOff(mono legato playing). In this case, default_fromkey must be valid.
    ///
    /// Typically in poly the function is called with default_fromkey INVALID_NOTE.
    /// Typically in mono the function is called with default_fromkey INVALID_NOTE.
    ///
    /// (The result is a `char`, signed here as on x86 and Apple platforms:
    /// INVALID_NOTE comes back as -1.)
    fn fluid_synth_get_fromkey_portamento_legato(
        &mut self,
        chan: usize,
        mut default_fromkey: i32,
    ) -> i8 {
        let channel = &mut self.channel[chan];
        let ptc = channel.get_cc(PORTAMENTO_CTRL) as u8;

        if fluid_channel_is_valid_note(ptc) {
            /* CC PTC has been received */
            channel.clear_portamento(); /* clears the CC PTC receive */
            self.fromkey_portamento = ptc as i32; /* returns fromkey portamento */

            /* returns fromkey legato */
            if !fluid_channel_is_valid_note(default_fromkey) {
                default_fromkey = ptc as i32;
            }
        } else {
            /* determines and returns fromkey portamento */
            let mut fromkey_portamento: u8 = INVALID_NOTE;

            if channel.portamento() {
                /* Portamento when Portamento pedal is On */
                /* 'fromkey portamento'is determined from the portamento mode
                and the most recent note played (prev_note)*/
                let portamentomode = channel.portamentomode;

                if fluid_channel_is_valid_note(default_fromkey) {
                    fromkey_portamento = default_fromkey as u8; /* on each note */
                } else {
                    fromkey_portamento = channel.prev_note(); /* on each note */
                }

                if portamentomode == FLUID_CHANNEL_PORTAMENTO_MODE_LEGATO_ONLY {
                    /* Mode portamento:legato only */
                    if channel.mode & FLUID_CHANNEL_LEGATO_PLAYING == 0 {
                        fromkey_portamento = INVALID_NOTE;
                    }
                } else if portamentomode == FLUID_CHANNEL_PORTAMENTO_MODE_STACCATO_ONLY {
                    /* Mode portamento:staccato only */
                    if channel.mode & FLUID_CHANNEL_LEGATO_PLAYING != 0 {
                        fromkey_portamento = INVALID_NOTE;
                    }
                }

                /* else Mode portamento: on each note (staccato/legato) */
            }

            /* Returns fromkey portamento */
            self.fromkey_portamento = fromkey_portamento as i32;

            /* Determines and returns fromkey legato */
            if !fluid_channel_is_valid_note(default_fromkey) {
                /* in staccato (poly/Mono) returns INVALID_NOTE */
                /* In mono mode legato playing returns the note prior most
                recent note played */
                if channel.is_playing_mono() && (channel.mode & FLUID_CHANNEL_LEGATO_PLAYING) != 0 {
                    default_fromkey = channel.prev_note() as i32; /* note prior last note */
                }

                /* In poly mode legato playing, actually we don't want playing legato.
                So returns INVALID_NOTE */
            }
        }

        default_fromkey as i8 /* Returns legato fromkey */
    }

    /*****************************************************************************
      noteon - noteoff functions in Mono mode
    ******************************************************************************/
    /// Translation of `fluid_synth_noteon_mono_LOCAL()`: plays a noteon
    /// event for a Synth instance in "monophonic playing" state.
    /// Please see the description above about "monophonic playing".
    ///
    /// ```text
    ///                         _______________
    ///                ________|    noteon     |
    ///               | legato |_______________|
    ///               |
    ///               v
    ///    +--------------------------------------------------+
    ///    | monophonic list legato detector                  |
    ///    | - On noteOn n2, if a previous note n1 exists,    |
    ///    |   there is a legato detection with n1. Then      |
    ///    |   fluid_synth_noteon_monopoly_legato() is called |
    ///    |   to play legato from n1 to n2.                  |
    ///    | - Otherwise, fluid_synth_noteon_mono_staccato()  |
    ///    |   is called to play n2 in staccato manner        |
    ///    +--------------------------------------------------+
    /// ```
    ///
    /// @param chan  MIDI channel number (0 to MIDI channel count - 1)
    /// @param key MIDI note number (0-127)
    /// @param vel MIDI velocity (0-127)
    /// @return FLUID_OK on success, FLUID_FAILED otherwise
    ///
    /// Note: The monophonic list is updated in legato_on order.
    pub(crate) fn fluid_synth_noteon_mono_local(&mut self, chan: i32, key: i32, vel: i32) -> i32 {
        let channel = &mut self.channel[chan as usize];

        /* Adds the note into the monophonic list */
        channel.add_monolist(key as u8, vel as u8, 0);

        /* in Breath Sync mode, the noteon triggering is postponed
        until the musician starts blowing in the breath controller */
        if (channel.mode & FLUID_CHANNEL_BREATH_SYNC) == 0 || channel.breath_msb() {
            /* legato/staccato playing detection */
            if channel.mode & FLUID_CHANNEL_LEGATO_PLAYING != 0 {
                /* legato playing */
                /* legato from prev_note to key */
                /* the voices from prev_note key number are to be used to play key number */
                /* fromkey must be valid */
                let prev = channel.prev_note() as i32;
                self.fluid_synth_noteon_monopoly_legato(chan, prev, key, vel)
            } else {
                /* staccato playing */
                self.fluid_synth_noteon_mono_staccato(chan, key, vel)
            }
        } else {
            FLUID_OK
        }
    }

    /// Translation of `fluid_synth_noteoff_mono_LOCAL()`: plays a noteoff
    /// event for a Synth instance in "monophonic playing" state.
    /// Please see the description above about "monophonic playing"
    ///
    /// ```text
    ///                         _______________
    ///                        |   noteoff     |
    ///                        |_______________|
    ///                               |
    ///                               v
    ///    +--------------------------------------------------+
    ///    | monophonic list legato detector                  |
    ///    | - On noteOff of the running note n2, if a        |
    ///    |   previous note n1 exists, there is a legato     |
    ///    |   detection from n2 to n1. Then                  |
    ///    |   fluid_synth_noteon_monopoly_legato() is called |
    ///    |   to play legato from n2 to n1.                  |
    ///    | - Otherwise, fluid_synth_noteoff_monopoly()      |
    ///    |   is called to play noteoff on n2                |
    ///    +--------------------------------------------------+
    /// ```
    ///
    /// @param chan  MIDI channel number (0 to MIDI channel count - 1)
    /// @param key MIDI note number (0-127)
    /// @return FLUID_OK on success, FLUID_FAILED otherwise
    ///
    /// Note: The monophonic list is updated in legato_off order.
    pub(crate) fn fluid_synth_noteoff_mono_local(&mut self, chan: i32, key: i32) -> i32 {
        let status;
        let mut i_prev: i32 = 0;
        let channel = &mut self.channel[chan as usize];
        /* searching the note in the monophonic list */
        let i = channel.search_monolist(key as u8, &mut i_prev);

        if i >= 0 {
            /* the note is in the monophonic list */
            /* Removes the note from the monophonic list */
            channel.remove_monolist(i, &mut i_prev);

            /* in Breath Sync mode, the noteoff triggering is done
            if the musician is blowing in the breath controller */
            if (channel.mode & FLUID_CHANNEL_BREATH_SYNC) == 0 || channel.breath_msb() {
                /* legato playing detection */
                if channel.mode & FLUID_CHANNEL_LEGATO_PLAYING != 0 && channel.preset.is_some() {
                    /* the list contains others notes */
                    if i_prev >= 0 {
                        /* legato playing detection on noteoff */
                        /* legato from key to i_prev key */
                        /* the voices from key number are to be used to
                        play i_prev key number. */
                        let note = channel.monolist[i_prev as usize].note as i32;
                        let vel = channel.monolist[i_prev as usize].vel as i32;
                        status = self.fluid_synth_noteon_monopoly_legato(chan, key, note, vel);
                    }
                    /* else the note doesn't need to be played off */
                    else {
                        status = FLUID_OK;
                    }
                } else {
                    /* the monophonic list is empty */
                    /* plays the monophonic note noteoff and eventually held
                    by sustain/sostenuto */
                    status = self.fluid_synth_noteoff_monopoly(chan, key, true);
                }
            } else {
                status = FLUID_OK;
            }
        } else {
            /* the note is not found in the list so the note was
            played On when the channel was in polyphonic playing */
            /* plays the noteoff as for polyphonic  */
            status = self.fluid_synth_noteoff_monopoly(chan, key, false);
        }

        status
    }

    /*----------------------------------------------------------------------------
     staccato playing
    -----------------------------------------------------------------------------*/
    /// Translation of `fluid_synth_noteon_mono_staccato()`: plays noteon
    /// for a monophonic note in staccato manner.
    /// Please see the description above about "monophonic playing".
    ///                         _______________
    ///                        |    noteon     |
    ///                        |_______________|
    ///                               |
    ///                               v
    ///            __________________________________________
    ///           | fluid_synth_noteon_mono_staccato()       |
    ///           | - Before playing a new note, if a        |
    ///           |   previous monophonic note is currently  |
    ///           |   sustained it needs to be released.     |
    ///           | - Get 'fromkey portamento'               |
    ///           | - The note needs to be played by voices  |
    ///           |   allocation                             |
    ///           |__________________________________________|
    ///
    /// @param chan  MIDI channel number (0 to MIDI channel count - 1)
    /// @param key MIDI note number (0-127)
    /// @param vel MIDI velocity (0-127)
    /// @return FLUID_OK on success, FLUID_FAILED otherwise
    pub(crate) fn fluid_synth_noteon_mono_staccato(
        &mut self,
        chan: i32,
        key: i32,
        vel: i32,
    ) -> i32 {
        /* Before playing a new note, if a previous monophonic note is currently
        sustained it needs to be released */
        let sustained = self.channel[chan as usize].key_mono_sustained as i32;
        self.fluid_synth_release_voice_on_same_note_local(chan, sustained);
        /* Get possible 'fromkey portamento'   */
        self.fluid_synth_get_fromkey_portamento_legato(chan as usize, INVALID_NOTE as i32);
        /* The note needs to be played by voices allocation  */
        self.fluid_preset_noteon(chan, key, vel)
    }

    /// `fluid_preset_noteon(channel->preset, synth, chan, key, vel)`.
    fn fluid_preset_noteon(&mut self, chan: i32, key: i32, vel: i32) -> i32 {
        let Some(preset) = self.channel[chan as usize].preset.clone() else {
            // FIXME (upstream): reached without a preset (through breath
            // sync mode, which only the API can set), this dereferences
            // NULL.
            return FLUID_FAILED;
        };
        self.fluid_defpreset_noteon(&preset.data, chan, key, vel)
    }

    /// Translation of `fluid_synth_noteoff_monopoly()`: plays noteoff for
    /// a polyphonic or monophonic note.
    /// Please see the description above about "monophonic playing".
    ///
    /// ```text
    ///                         _______________
    ///                        |   noteoff     |
    ///                        |_______________|
    ///                               |
    ///                               v
    ///            __________________________________________
    ///           | fluid_synth_noteoff_monopoly()           |
    ///           | - If the note is sustained (by sustain   |
    ///           |   or sostenuto pedal), memorize it.      |
    ///           | - Otherwise the note is played off.      |
    ///           |__________________________________________|
    /// ```
    ///
    /// @param chan  MIDI channel number (0 to MIDI channel count - 1)
    /// @param key MIDI note number (0-127)
    /// @param Mono, 1 noteoff on monophonic note.
    ///              0 noteoff on polyphonic note.
    /// @return FLUID_OK on success, FLUID_FAILED otherwise
    ///
    /// Note: On return, on monophonic, possible sustained note is memorized in
    /// key_mono_sustained. Memorization is done here on noteOff.
    pub(crate) fn fluid_synth_noteoff_monopoly(&mut self, chan: i32, key: i32, mono: bool) -> i32 {
        let mut status = FLUID_FAILED;

        /* Key_sustained is prepared to return no note sustained (INVALID_NOTE) */
        if mono {
            self.channel[chan as usize].key_mono_sustained = INVALID_NOTE; /* no mono note sustained */
        }

        /* noteoff for all voices with same chan and same key */
        for i in 0..self.polyphony as usize {
            let voice = &self.voice[i];

            if fluid_voice_is_on(voice)
                && fluid_voice_get_channel(voice) == chan
                && fluid_voice_get_key(voice) == key
            {
                if self.verbose != 0 {
                    let mut used_voices = 0;

                    for k in 0..self.polyphony as usize {
                        if !available(&self.voice[k]) {
                            used_voices += 1;
                        }
                    }

                    flog!(
                        FLUID_INFO,
                        "noteoff\t{}\t{}\t{}\t{:05}\t{:.3}\t{}",
                        fluid_voice_get_channel(voice),
                        fluid_voice_get_key(voice),
                        0,
                        voice.id,
                        (crate::utils::sys::fluid_curtime().wrapping_sub(self.start)) as f32
                            / 1000.0f32,
                        used_voices
                    );
                } /* if verbose */

                let (voice, mut env) = self.split_voice(i);
                fluid_voice_noteoff(voice, &mut env);

                /* noteoff on monophonic note */
                /* Key memorization if the note is sustained  */
                if mono && (fluid_voice_is_sustained(voice) || fluid_voice_is_sostenuto(voice)) {
                    self.channel[chan as usize].key_mono_sustained = key as u8;
                }

                status = FLUID_OK;
            } /* if voice on */
        } /* for all voices */

        status
    }

    /*----------------------------------------------------------------------------
     legato playing
    -----------------------------------------------------------------------------*/
    /// Translation of `fluid_synth_noteon_monopoly_legato()`: plays noteon
    /// for a monophonic note played legato.
    /// Please see the description above about "monophonic playing".
    ///
    ///
    /// ```text
    ///                         _______________
    ///                        |    noteon     |
    ///                        |_______________|
    ///                               |
    ///                               v
    ///            __________________________________________
    ///           | fluid_synth_noteon_monopoly_legato()     |
    ///           | - Get 'fromkey portamento' and 'fromkey  |
    ///           |   legato'                                |
    ///           | - Running voices are changed to play     |
    ///           |   'tokey'                                |
    ///           | - For each running voices, apply legato  |
    ///           |   mode and portamento                    |
    ///           |__________________________________________|
    /// ```
    ///
    /// @param chan  MIDI channel number (0 to MIDI channel count - 1)
    /// @param fromkey MIDI note number (0-127)
    /// @param tokey MIDI note number (0-127)
    /// @param vel MIDI velocity (0-127)
    /// @return FLUID_OK on success, FLUID_FAILED otherwise
    ///
    /// Note: The voices with key 'fromkey' are to be used to play key 'tokey'.
    /// The function is able to play legato through Preset Zone(s) (PZ) and
    /// Instrument Zone(s) (IZ) as far as possible.
    /// When key tokey is outside the current Instrument Zone,
    /// Preset Zone, current 'fromkey' voices are released. If necessary new
    /// voices are restarted when tokey enters inside new Instrument(s) Zones,
    /// Preset Zone(s).
    /// More information in FluidPolyMono-0004.pdf chapter 4.7 (Appendices).
    pub(crate) fn fluid_synth_noteon_monopoly_legato(
        &mut self,
        chan: i32,
        fromkey: i32,
        tokey: i32,
        vel: i32,
    ) -> i32 {
        let legatomode = self.channel[chan as usize].legatomode;
        /* Gets possible 'fromkey portamento' and possible 'fromkey legato' note  */
        let fromkey = self.fluid_synth_get_fromkey_portamento_legato(chan as usize, fromkey) as i32;

        if fluid_channel_is_valid_note(fromkey) {
            for i in 0..self.polyphony as usize {
                /* searching fromkey voices: only those who don't have 'note off' */
                let voice = &self.voice[i];

                if fluid_voice_is_on(voice)
                    && fluid_voice_get_channel(voice) == chan
                    && fluid_voice_get_key(voice) == fromkey
                {
                    let zone_range = voice.zone_range.clone();

                    /* Ignores voice when there is no instrument zone (i.e no zone_range). Otherwise
                    checks if tokey is inside the range of the running voice */
                    if let Some(zone_range) =
                        zone_range.filter(|z| fluid_zone_inside_range(z, tokey, vel))
                    {
                        match legatomode {
                            FLUID_CHANNEL_LEGATO_MODE_RETRIGGER => {
                                /* mode 0 */
                                let (voice, mut env) = self.split_voice(i);
                                fluid_voice_release(voice, &mut env); /* normal release */
                            }

                            FLUID_CHANNEL_LEGATO_MODE_MULTI_RETRIGGER => {
                                /* mode 1 */
                                /* Skip in attack section */
                                let fromkey_portamento = self.fromkey_portamento;
                                let (voice, mut env) = self.split_voice(i);
                                fluid_voice_update_multi_retrigger_attack(
                                    voice, &mut env, tokey, vel,
                                );

                                /* Starts portamento if enabled */
                                if fluid_channel_is_valid_note(fromkey_portamento) {
                                    /* Sends portamento parameters to the voice dsp */
                                    fluid_voice_update_portamento(
                                        voice,
                                        &mut env,
                                        fromkey_portamento,
                                        tokey,
                                    );
                                }

                                /* The voice is now used to play tokey in legato manner */
                                /* Marks this Instrument Zone to be ignored during next
                                fluid_preset_noteon() */
                                zone_range.ignore.store(true, Ordering::Relaxed);
                            }

                            _ => {
                                /* Invalid mode: this should never happen */
                                flog!(FLUID_WARN, "Failed to execute legato mode: {}", legatomode);
                                return FLUID_FAILED;
                            }
                        }
                    } else {
                        /* tokey note is outside the voice range, so the voice is released */
                        let (voice, mut env) = self.split_voice(i);
                        fluid_voice_release(voice, &mut env);
                    }
                }
            }
        }

        /* May be,tokey will enter in new others Insrument Zone(s),Preset Zone(s), in
        this case it needs to be played by voices allocation  */
        self.fluid_preset_noteon(chan, tokey, vel)
    }
}
