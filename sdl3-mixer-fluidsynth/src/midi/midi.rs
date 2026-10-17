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
// Modified 2026-10-17: translated into Rust from the parts of
// fluid_midi.c and fluid_midi.h in SDL_mixer 3.3.0's FluidSynth 2.5.7
// (see NOTICE) that SDL_mixer reaches: the MIDI file parser, MIDI events
// and tracks, and the MIDI file player driven by a sample timer with
// songs added from memory. The player's state is behind its own mutex
// (locked after the synth's), with the status, stopping and seek fields
// atomic as upstream. Its playback callback is always
// fluid_synth_handle_midi_event() (upstream's default) and there is no
// tick callback; the system timer ("player.timing-source" "system", which
// needs a thread), playing files by name, the tempo setters and getters
// and the realtime MIDI parser are left out.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use crate::fluidsynth_h::*;
use crate::fluidsynth_priv::c_double_to_int;
use crate::synth::synth::{FluidSynthInner, Synth};
use crate::synth::voice::{fluid_voice_get_channel, fluid_voice_get_key, fluid_voice_is_on};
use crate::utils::settings::SettingsInner;
use crate::utils::sys::{flog, FLUID_DBG, FLUID_ERR, FLUID_INFO, FLUID_PANIC, FLUID_WARN};

/***************************************************************
 *
 *                   CONSTANTS & ENUM
 */

pub(crate) const MAX_NUMBER_OF_TRACKS: usize = 128;
pub(crate) const MAX_NUMBER_OF_CHANNELS: usize = 16;

/* enum fluid_midi_event_type */
/* channel messages */
pub(crate) const NOTE_OFF: i32 = 0x80;
pub(crate) const NOTE_ON: i32 = 0x90;
pub(crate) const KEY_PRESSURE: i32 = 0xa0;
pub(crate) const CONTROL_CHANGE: i32 = 0xb0;
pub(crate) const PROGRAM_CHANGE: i32 = 0xc0;
pub(crate) const CHANNEL_PRESSURE: i32 = 0xd0;
pub(crate) const PITCH_BEND: i32 = 0xe0;
/* system exclusive */
pub(crate) const MIDI_SYSEX: i32 = 0xf0;
/* system common - never in midi files */
#[allow(dead_code)]
pub(crate) const MIDI_TIME_CODE: i32 = 0xf1;
#[allow(dead_code)]
pub(crate) const MIDI_SONG_POSITION: i32 = 0xf2;
#[allow(dead_code)]
pub(crate) const MIDI_SONG_SELECT: i32 = 0xf3;
#[allow(dead_code)]
pub(crate) const MIDI_TUNE_REQUEST: i32 = 0xf6;
pub(crate) const MIDI_EOX: i32 = 0xf7;
/* system realtime - never in midi files */
#[allow(dead_code)]
pub(crate) const MIDI_SYNC: i32 = 0xf8;
#[allow(dead_code)]
pub(crate) const MIDI_TICK: i32 = 0xf9;
#[allow(dead_code)]
pub(crate) const MIDI_START: i32 = 0xfa;
#[allow(dead_code)]
pub(crate) const MIDI_CONTINUE: i32 = 0xfb;
#[allow(dead_code)]
pub(crate) const MIDI_STOP: i32 = 0xfc;
#[allow(dead_code)]
pub(crate) const MIDI_ACTIVE_SENSING: i32 = 0xfe;
pub(crate) const MIDI_SYSTEM_RESET: i32 = 0xff;
/* meta event - for midi files only */
pub(crate) const MIDI_META_EVENT: i32 = 0xff;

/* enum fluid_midi_control_change */
pub(crate) const BANK_SELECT_MSB: i32 = 0x00;
pub(crate) const MODULATION_MSB: i32 = 0x01;
pub(crate) const BREATH_MSB: i32 = 0x02;
#[allow(dead_code)]
pub(crate) const FOOT_MSB: i32 = 0x04;
pub(crate) const PORTAMENTO_TIME_MSB: i32 = 0x05;
pub(crate) const DATA_ENTRY_MSB: i32 = 0x06;
pub(crate) const VOLUME_MSB: i32 = 0x07;
#[allow(dead_code)]
pub(crate) const BALANCE_MSB: i32 = 0x08;
pub(crate) const PAN_MSB: i32 = 0x0A;
pub(crate) const EXPRESSION_MSB: i32 = 0x0B;
#[allow(dead_code)]
pub(crate) const EFFECTS1_MSB: i32 = 0x0C;
#[allow(dead_code)]
pub(crate) const EFFECTS2_MSB: i32 = 0x0D;
#[allow(dead_code)]
pub(crate) const GPC1_MSB: i32 = 0x10; /* general purpose controller */
#[allow(dead_code)]
pub(crate) const GPC2_MSB: i32 = 0x11;
#[allow(dead_code)]
pub(crate) const GPC3_MSB: i32 = 0x12;
#[allow(dead_code)]
pub(crate) const GPC4_MSB: i32 = 0x13;
pub(crate) const BANK_SELECT_LSB: i32 = 0x20;
#[allow(dead_code)]
pub(crate) const MODULATION_WHEEL_LSB: i32 = 0x21;
#[allow(dead_code)]
pub(crate) const BREATH_LSB: i32 = 0x22;
#[allow(dead_code)]
pub(crate) const FOOT_LSB: i32 = 0x24;
pub(crate) const PORTAMENTO_TIME_LSB: i32 = 0x25;
pub(crate) const DATA_ENTRY_LSB: i32 = 0x26;
#[allow(dead_code)]
pub(crate) const VOLUME_LSB: i32 = 0x27;
#[allow(dead_code)]
pub(crate) const BALANCE_LSB: i32 = 0x28;
#[allow(dead_code)]
pub(crate) const PAN_LSB: i32 = 0x2A;
#[allow(dead_code)]
pub(crate) const EXPRESSION_LSB: i32 = 0x2B;
#[allow(dead_code)]
pub(crate) const EFFECTS1_LSB: i32 = 0x2C;
#[allow(dead_code)]
pub(crate) const EFFECTS2_LSB: i32 = 0x2D;
#[allow(dead_code)]
pub(crate) const GPC1_LSB: i32 = 0x30;
#[allow(dead_code)]
pub(crate) const GPC2_LSB: i32 = 0x31;
#[allow(dead_code)]
pub(crate) const GPC3_LSB: i32 = 0x32;
#[allow(dead_code)]
pub(crate) const GPC4_LSB: i32 = 0x33;
pub(crate) const SUSTAIN_SWITCH: i32 = 0x40;
pub(crate) const PORTAMENTO_SWITCH: i32 = 0x41;
pub(crate) const SOSTENUTO_SWITCH: i32 = 0x42;
#[allow(dead_code)]
pub(crate) const SOFT_PEDAL_SWITCH: i32 = 0x43;
pub(crate) const LEGATO_SWITCH: i32 = 0x44;
#[allow(dead_code)]
pub(crate) const HOLD2_SWITCH: i32 = 0x45;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL1: i32 = 0x46;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL2: i32 = 0x47;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL3: i32 = 0x48;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL4: i32 = 0x49;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL5: i32 = 0x4A;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL6: i32 = 0x4B;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL7: i32 = 0x4C;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL8: i32 = 0x4D;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL9: i32 = 0x4E;
#[allow(dead_code)]
pub(crate) const SOUND_CTRL10: i32 = 0x4F;
#[allow(dead_code)]
pub(crate) const GPC5: i32 = 0x50;
#[allow(dead_code)]
pub(crate) const GPC6: i32 = 0x51;
#[allow(dead_code)]
pub(crate) const GPC7: i32 = 0x52;
#[allow(dead_code)]
pub(crate) const GPC8: i32 = 0x53;
pub(crate) const PORTAMENTO_CTRL: i32 = 0x54;
pub(crate) const EFFECTS_DEPTH1: i32 = 0x5B;
#[allow(dead_code)]
pub(crate) const EFFECTS_DEPTH2: i32 = 0x5C;
pub(crate) const EFFECTS_DEPTH3: i32 = 0x5D;
#[allow(dead_code)]
pub(crate) const EFFECTS_DEPTH4: i32 = 0x5E;
#[allow(dead_code)]
pub(crate) const EFFECTS_DEPTH5: i32 = 0x5F;
#[allow(dead_code)]
pub(crate) const DATA_ENTRY_INCR: i32 = 0x60;
#[allow(dead_code)]
pub(crate) const DATA_ENTRY_DECR: i32 = 0x61;
pub(crate) const NRPN_LSB: i32 = 0x62;
pub(crate) const NRPN_MSB: i32 = 0x63;
pub(crate) const RPN_LSB: i32 = 0x64;
pub(crate) const RPN_MSB: i32 = 0x65;
pub(crate) const ALL_SOUND_OFF: i32 = 0x78;
pub(crate) const ALL_CTRL_OFF: i32 = 0x79;
#[allow(dead_code)]
pub(crate) const LOCAL_CONTROL: i32 = 0x7A;
pub(crate) const ALL_NOTES_OFF: i32 = 0x7B;
pub(crate) const OMNI_OFF: i32 = 0x7C;
pub(crate) const OMNI_ON: i32 = 0x7D;
pub(crate) const POLY_OFF: i32 = 0x7E;
pub(crate) const POLY_ON: i32 = 0x7F;

/* General MIDI RPN event numbers (LSB, MSB = 0) */
/* enum midi_rpn_event */
pub(crate) const RPN_PITCH_BEND_RANGE: i32 = 0x00;
pub(crate) const RPN_CHANNEL_FINE_TUNE: i32 = 0x01;
pub(crate) const RPN_CHANNEL_COARSE_TUNE: i32 = 0x02;
pub(crate) const RPN_TUNING_PROGRAM_CHANGE: i32 = 0x03;
pub(crate) const RPN_TUNING_BANK_SELECT: i32 = 0x04;
pub(crate) const RPN_MODULATION_DEPTH_RANGE: i32 = 0x05;

/* enum midi_meta_event */
pub(crate) const MIDI_TEXT: i32 = 0x01;
pub(crate) const MIDI_COPYRIGHT: i32 = 0x02;
pub(crate) const MIDI_TRACK_NAME: i32 = 0x03;
pub(crate) const MIDI_INST_NAME: i32 = 0x04;
pub(crate) const MIDI_LYRIC: i32 = 0x05;
pub(crate) const MIDI_MARKER: i32 = 0x06;
pub(crate) const MIDI_CUE_POINT: i32 = 0x07;
pub(crate) const MIDI_EOT: i32 = 0x2f;
pub(crate) const MIDI_SET_TEMPO: i32 = 0x51;
pub(crate) const MIDI_SMPTE_OFFSET: i32 = 0x54;
pub(crate) const MIDI_TIME_SIGNATURE: i32 = 0x58;
pub(crate) const MIDI_KEY_SIGNATURE: i32 = 0x59;
pub(crate) const MIDI_SEQUENCER_EVENT: i32 = 0x7f;

/* MIDI SYSEX useful manufacturer values */
/* enum midi_sysex_manuf */
pub(crate) const MIDI_SYSEX_MANUF_ROLAND: i32 = 0x41; /*< Roland manufacturer ID */
pub(crate) const MIDI_SYSEX_MANUF_YAMAHA: i32 = 0x43;
pub(crate) const MIDI_SYSEX_UNIV_NON_REALTIME: i32 = 0x7E; /*< Universal non realtime message */
pub(crate) const MIDI_SYSEX_UNIV_REALTIME: i32 = 0x7F; /*< Universal realtime message */

pub(crate) const MIDI_SYSEX_DEVICE_ID_ALL: i32 = 0x7F; /*< Device ID used in SYSEX messages to indicate all devices */

/* SYSEX sub-ID #1 which follows device ID */
pub(crate) const MIDI_SYSEX_MIDI_TUNING_ID: i32 = 0x08; /*< Sysex sub-ID #1 for MIDI tuning messages */
pub(crate) const MIDI_SYSEX_GM_ID: i32 = 0x09; /*< Sysex sub-ID #1 for General MIDI messages */
pub(crate) const MIDI_SYSEX_GS_ID: i32 = 0x42; /*< Model ID (GS) serving as sub-ID #1 for GS messages*/
pub(crate) const MIDI_SYSEX_XG_ID: i32 = 0x4C; /*< Model ID (XG) serving as sub-ID #1 for XG messages*/

/* SYSEX tuning message IDs. */
/* enum midi_sysex_tuning_msg_id */
pub(crate) const MIDI_SYSEX_TUNING_BULK_DUMP_REQ: i32 = 0x00; /*< Bulk tuning dump request (non-realtime) */
#[allow(dead_code)]
pub(crate) const MIDI_SYSEX_TUNING_BULK_DUMP: i32 = 0x01; /*< Bulk tuning dump response (non-realtime) */
pub(crate) const MIDI_SYSEX_TUNING_NOTE_TUNE: i32 = 0x02; /*< Tuning note change message (realtime) */
pub(crate) const MIDI_SYSEX_TUNING_BULK_DUMP_REQ_BANK: i32 = 0x03; /*< Bulk tuning dump request (with bank, non-realtime) */
#[allow(dead_code)]
pub(crate) const MIDI_SYSEX_TUNING_BULK_DUMP_BANK: i32 = 0x04; /*< Bulk tuning dump response (with bank, non-realtime) */
#[allow(dead_code)]
pub(crate) const MIDI_SYSEX_TUNING_OCTAVE_DUMP_1BYTE: i32 = 0x05; /*< Octave tuning dump using 1 byte values (non-realtime) */
#[allow(dead_code)]
pub(crate) const MIDI_SYSEX_TUNING_OCTAVE_DUMP_2BYTE: i32 = 0x06; /*< Octave tuning dump using 2 byte values (non-realtime) */
pub(crate) const MIDI_SYSEX_TUNING_NOTE_TUNE_BANK: i32 = 0x07; /*< Tuning note change message (with bank, realtime/non-realtime) */
pub(crate) const MIDI_SYSEX_TUNING_OCTAVE_TUNE_1BYTE: i32 = 0x08; /*< Octave tuning message using 1 byte values (realtime/non-realtime) */
pub(crate) const MIDI_SYSEX_TUNING_OCTAVE_TUNE_2BYTE: i32 = 0x09; /*< Octave tuning message using 2 byte values (realtime/non-realtime) */

/* General MIDI sub-ID #2 */
pub(crate) const MIDI_SYSEX_GM_ON: i32 = 0x01; /*< Enable GM mode */
#[allow(dead_code)]
pub(crate) const MIDI_SYSEX_GM_OFF: i32 = 0x02; /*< Disable GM mode */
pub(crate) const MIDI_SYSEX_GM2_ON: i32 = 0x03; /*< Enable GM2 mode */
pub(crate) const MIDI_SYSEX_GS_DT1: i32 = 0x12; /*< GS DT1 command */

/***************************************************************
 *
 *         TYPE DEFINITIONS & FUNCTION DECLARATIONS
 */

/// Translation of `fluid_midi_event_t`: a MIDI event (the `next` link
/// is the position in its track's event list).
#[derive(Clone, Debug, Default)]
pub struct FluidMidiEvent {
    /// Pointer parameter (for SYSEX data), size is stored to param1 (the
    /// C param2 flag of a dynamic pointer is the ownership of the box)
    pub(crate) paramptr: Option<Box<[i8]>>,
    pub(crate) dtime: u32, /* Delay (ticks) between this and previous event. midi tracks. */
    pub(crate) param1: u32, /* First parameter */
    pub(crate) param2: u32, /* Second parameter */
    pub(crate) type_: u8,  /* MIDI event type */
    pub(crate) channel: u8, /* MIDI channel */
}

/// Translation of `fluid_track_t`: `cur` is an index into `events`
/// (`events.len()` for the C NULL).
struct FluidTrack {
    #[allow(dead_code)]
    name: Option<String>,
    num: i32,
    events: Vec<FluidMidiEvent>,
    cur: usize,
    ticks: u32,
}

impl FluidTrack {
    /// `fluid_track_eot(track)`
    #[inline]
    fn eot(&self) -> bool {
        self.cur >= self.events.len()
    }
}

/// Translation of `fluid_playlist_item`: used as the `data' elements of
/// the fluid_player.playlist. Represents a pre-loaded memory buffer (songs
/// given by file name are not translated).
struct FluidPlaylistItem {
    buffer: Vec<u8>, /* The MIDI file data (owned) */
}

/* range of tempo values */
#[allow(dead_code)]
const MIN_TEMPO_VALUE: f32 = 1.0f32;
#[allow(dead_code)]
const MAX_TEMPO_VALUE: f32 = 60000000.0f32;
/* range of tempo multiplier values */
#[allow(dead_code)]
const MIN_TEMPO_MULTIPLIER: f32 = 0.001f32;
#[allow(dead_code)]
const MAX_TEMPO_MULTIPLIER: f32 = 1000.0f32;

/// The atomic fields of `fluid_player_t`, shared with the player's
/// sample timer, and the rest of it behind a mutex.
struct FluidPlayerShared {
    status: AtomicI32,
    stopping: AtomicI32, /* Flag for sending all_notes_off when player is stopped */
    seek_ticks: AtomicI32, /* new position in tempo ticks (midi ticks) for seeking */
    state: Mutex<FluidPlayerState>,
}

/// Translation of the non-atomic fields of `fluid_player_t`.
struct FluidPlayerState {
    track: Vec<FluidTrack>, /* ntracks is its length */

    loop_: i32, /* -1 = loop infinitely, otherwise times left to loop the playlist */
    playlist: Vec<FluidPlaylistItem>, /* List of fluid_playlist_item* objects */
    currentfile: Option<usize>, /* points to an item in files, or NULL if not playing */

    reset_synth_between_songs: bool, /* 1 if system reset should be sent to the synth between songs. */
    start_ticks: i32,                /* the number of tempo ticks passed at the last tempo change */
    cur_ticks: i32,                  /* the number of tempo ticks passed */
    begin_msec: i32,                 /* the time (msec) of the beginning of the file */
    start_msec: i32,                 /* the start time of the last tempo change */
    cur_msec: u32,                   /* the current time */
    end_msec: i32, /* when >=0, playback is extended until this time (for, e.g., reverb) */
    end_pedals_disabled: bool, /* 1 once the pedals have been released after the last midi event, 0 otherwise */
    /* sync mode: indicates the tempo mode the player is driven by (see fluid_player_set_tempo()):
       1, the player is driven by internal tempo (miditempo). This is the default.
       0, the player is driven by external tempo (exttempo)
    */
    sync_mode: i32,
    /* miditempo: internal tempo coming from MIDI file tempo change events
      (in micro seconds per quarter note)
    */
    miditempo: i32, /* as indicated by MIDI SetTempo: n 24th of a usec per midi-clock. bravo! */
    /* exttempo: external tempo set by fluid_player_set_tempo() (in micro seconds per quarter note) */
    exttempo: i32,
    /* multempo: tempo multiplier set by fluid_player_set_tempo() */
    multempo: f32,
    deltatime: f32, /* milliseconds per midi tick. depends on current tempo mode (see sync_mode) */
    division: u32,

    channel_isplaying: [bool; MAX_NUMBER_OF_CHANNELS], /* flags indicating channels on which notes have played */
}

const FLUID_PLAYER_STOP_GRACE_MS: u32 = 2000;

/// Translation of `fluid_midi_file`.
struct FluidMidiFile<'a> {
    buffer: &'a [u8], /* Entire contents of MIDI file (borrowed) */
    buf_len: i32,     /* Length of buffer, in bytes */
    buf_pos: i32,     /* Current read position in contents buffer */
    eof: bool,        /* The "end of file" condition */
    running_status: i32,
    c: i32,
    type_: i32,
    ntracks: i32,
    #[allow(dead_code)]
    uses_smpte: i32,
    #[allow(dead_code)]
    smpte_fps: u32,
    #[allow(dead_code)]
    smpte_res: u32,
    division: u32, /* If uses_SMPTE == 0 then division is
                   ticks per beat (quarter-note) */
    tracklen: i32,
    trackpos: i32,
    eot: bool,
    varlen: i32,
    dtime: i32,
}

/***************************************************************
 *
 *                      MIDIFILE
 */

/// Translation of `new_fluid_midi_file()`: return a new MIDI file handle
/// for parsing an already-loaded MIDI file, or None on error.
fn new_fluid_midi_file(buffer: &[u8]) -> Option<FluidMidiFile<'_>> {
    if buffer.len() > i32::MAX as usize {
        flog!(
            FLUID_ERR,
            "Refusing to open a MIDI file which is bigger than 2GiB"
        );
        return None;
    }

    let mut mf = FluidMidiFile {
        buffer,
        buf_len: buffer.len() as i32,
        buf_pos: 0,
        eof: false,
        running_status: 0,
        c: -1,
        type_: 0,
        ntracks: 0,
        uses_smpte: 0,
        smpte_fps: 0,
        smpte_res: 0,
        division: 0,
        tracklen: 0,
        trackpos: 0,
        eot: false,
        varlen: 0,
        dtime: 0,
    };

    if fluid_midi_file_read_mthd(&mut mf) != FLUID_OK {
        return None;
    }

    Some(mf)
}

/// Translation of `fluid_midi_file_getc()`: gets the next byte in a MIDI
/// file, taking into account previous running status.
///
/// returns -1 if EOF or read error
fn fluid_midi_file_getc(mf: &mut FluidMidiFile<'_>) -> i32 {
    let c: u8;

    if mf.c >= 0 {
        c = mf.c as u8;
        mf.c = -1;
    } else {
        if mf.buf_pos >= mf.buf_len {
            mf.eof = true;
            return -1;
        }

        c = mf.buffer[mf.buf_pos as usize];
        mf.buf_pos += 1;
        mf.trackpos = mf.trackpos.wrapping_add(1);
    }

    c as i32
}

/// Translation of `fluid_midi_file_push()`: saves a byte to be returned
/// the next time fluid_midi_file_getc() is called, when it is necessary
/// according to running status.
fn fluid_midi_file_push(mf: &mut FluidMidiFile<'_>, c: i32) -> i32 {
    mf.c = c;
    FLUID_OK
}

/// Translation of `fluid_midi_file_read()` (into `buf`, whose length is
/// the C `len`; with `buf` None, it only moves as a read of `len` bytes
/// would).
fn fluid_midi_file_read(mf: &mut FluidMidiFile<'_>, buf: Option<&mut [u8]>, len: i32) -> i32 {
    let mut num = if len < mf.buf_len - mf.buf_pos {
        len
    } else {
        mf.buf_len - mf.buf_pos
    };

    if num != len {
        mf.eof = true;
    }

    if num < 0 {
        num = 0;
    }

    /* Note: Read bytes, even if there aren't enough, but only increment
     * trackpos if successful (emulates old behaviour of fluid_midi_file_read)
     */
    // (after a skip past the end, buf_pos is beyond it, and num 0)
    if let (Some(buf), true) = (buf, num > 0) {
        let pos = mf.buf_pos as usize;
        buf[..num as usize].copy_from_slice(&mf.buffer[pos..pos + num as usize]);
    }
    mf.buf_pos += num;

    if num == len {
        mf.trackpos = mf.trackpos.wrapping_add(num);
    }

    if num != len {
        FLUID_FAILED
    } else {
        FLUID_OK
    }
}

/// Translation of `fluid_midi_file_skip()`.
fn fluid_midi_file_skip(mf: &mut FluidMidiFile<'_>, skip: i32) -> i32 {
    let new_pos = mf.buf_pos.wrapping_add(skip);

    /* Mimic the behaviour of fseek: Error to seek past the start of file, but
     * OK to seek past end (this just puts it into the EOF state). */
    if new_pos < 0 {
        flog!(FLUID_ERR, "Failed to seek position in file");
        return FLUID_FAILED;
    }

    /* Clear the EOF flag, even if moved past the end of the file (this is
     * consistent with the behaviour of fseek). */
    mf.eof = false;
    mf.buf_pos = new_pos;
    FLUID_OK
}

/// Translation of `fluid_midi_file_eof()`.
fn fluid_midi_file_eof(mf: &FluidMidiFile<'_>) -> bool {
    /* Note: This does not simply test whether the file read pointer is past
     * the end of the file. It mimics the behaviour of feof by actually
     * testing the stateful EOF condition, which is set to TRUE if getc or
     * fread have attempted to read past the end (but not if they have
     * precisely reached the end), but reset to FALSE upon a successful seek.
     */
    mf.eof
}

/// Translation of `fluid_midi_file_read_mthd()`.
fn fluid_midi_file_read_mthd(mf: &mut FluidMidiFile<'_>) -> i32 {
    let mut mthd = [0u8; 14];

    if fluid_midi_file_read(mf, Some(&mut mthd), 14) != FLUID_OK {
        return FLUID_FAILED;
    }

    // (char is signed, as on x86)
    let m = |i: usize| mthd[i] as i8 as i32;

    if &mthd[0..4] != b"MThd" || m(7) != 6 || m(9) > 2 {
        flog!(
            FLUID_ERR,
            "Doesn't look like a MIDI file: invalid MThd header"
        );
        return FLUID_FAILED;
    }

    mf.type_ = m(9);
    if !(mf.type_ == 0 || mf.type_ == 1) {
        flog!(
            FLUID_ERR,
            "Sorry, but MIDI Format {} is not supported by this player",
            mf.type_
        );
        return FLUID_FAILED;
    }
    mf.ntracks = (((m(10) as u32) << 8) | (m(11) as u32)) as i32;

    if m(12) < 0 {
        mf.uses_smpte = 1;
        mf.smpte_fps = (-m(12)) as u32;
        mf.smpte_res = m(13) as u32;
        flog!(FLUID_ERR, "File uses SMPTE timing -- Not implemented yet");
        return FLUID_FAILED;
    } else {
        mf.uses_smpte = 0;
        mf.division = ((m(12) as u32) << 8) | ((m(13) as u32) & 0xff);
        flog!(FLUID_DBG, "Division={}", mf.division as i32);
    }

    FLUID_OK
}

/// Translation of `fluid_midi_file_load_tracks()`.
fn fluid_midi_file_load_tracks(mf: &mut FluidMidiFile<'_>, player: &mut FluidPlayerState) -> i32 {
    for i in 0..mf.ntracks {
        if fluid_midi_file_read_track(mf, player, i) != FLUID_OK {
            return FLUID_FAILED;
        }
    }

    FLUID_OK
}

/// Translation of `fluid_isasciistring()` (of the bytes before a NUL).
fn fluid_isasciistring(s: &[u8]) -> bool {
    /* From ctype.h */
    let fluid_isascii = |c: u8| (c & !0x7f) == 0;

    for &c in s.iter().take_while(|&&c| c != 0) {
        if !fluid_isascii(c) {
            return false;
        }
    }

    true
}

/// Translation of `fluid_getlength()`.
fn fluid_getlength(s: &[u8]) -> i64 {
    // (the int arithmetic of upstream, s[0] << 24 wrapping into the sign)
    (s[3] as i32 | ((s[2] as i32) << 8) | ((s[1] as i32) << 16) | ((s[0] as i32) << 24)) as i64
}

/// Translation of `fluid_midi_file_read_tracklen()`.
fn fluid_midi_file_read_tracklen(mf: &mut FluidMidiFile<'_>) -> i32 {
    let mut length = [0u8; 5];

    if fluid_midi_file_read(mf, Some(&mut length[..4]), 4) != FLUID_OK {
        return FLUID_FAILED;
    }

    mf.tracklen = fluid_getlength(&length) as i32;
    mf.trackpos = 0;
    mf.eot = false;
    FLUID_OK
}

/// Translation of `fluid_midi_file_eot()`.
fn fluid_midi_file_eot(mf: &FluidMidiFile<'_>) -> bool {
    mf.eot || (mf.trackpos >= mf.tracklen)
}

/// Translation of `fluid_midi_file_read_track()`.
fn fluid_midi_file_read_track(
    mf: &mut FluidMidiFile<'_>,
    player: &mut FluidPlayerState,
    num: i32,
) -> i32 {
    let mut id = [0u8; 5];
    let mut length = [0u8; 5];
    let mut found_track = false;

    if fluid_midi_file_read(mf, Some(&mut id[..4]), 4) != FLUID_OK {
        return FLUID_FAILED;
    }

    id[4] = b'\0';
    mf.dtime = 0;

    while !found_track {
        if !fluid_isasciistring(&id) {
            flog!(FLUID_ERR, "A non-ascii track header found, corrupt file");
            return FLUID_FAILED;
        } else if id[..id.iter().position(|&c| c == 0).unwrap_or(4)] == *b"MTrk" {
            found_track = true;

            if fluid_midi_file_read_tracklen(mf) != FLUID_OK {
                return FLUID_FAILED;
            }

            let mut track = new_fluid_track(num);

            while !fluid_midi_file_eot(mf) {
                if fluid_midi_file_read_event(mf, &mut track) != FLUID_OK {
                    return FLUID_FAILED;
                }
            }

            /* Skip remaining track data, if any */
            if mf.trackpos < mf.tracklen
                && fluid_midi_file_skip(mf, mf.tracklen - mf.trackpos) != FLUID_OK
            {
                return FLUID_FAILED;
            }

            if fluid_player_add_track(player, track) != FLUID_OK {
                return FLUID_FAILED;
            }
        } else {
            found_track = false;

            if fluid_midi_file_read(mf, Some(&mut length[..4]), 4) != FLUID_OK {
                return FLUID_FAILED;
            }

            let skip = fluid_getlength(&length) as i32;

            // FIXME (upstream): the loop reads no new chunk ID, so it reads
            // lengths and skips until it fails; a negative skip that stays
            // inside the file can make that loop forever. Fail on it (the
            // loop could only end in failure anyway).
            if skip < 0 {
                flog!(FLUID_ERR, "Failed to seek position in file");
                return FLUID_FAILED;
            }

            /* fseek(mf->fp, skip, SEEK_CUR); */
            if fluid_midi_file_skip(mf, skip) != FLUID_OK {
                return FLUID_FAILED;
            }
        }
    }

    if fluid_midi_file_eof(mf) {
        flog!(FLUID_ERR, "Unexpected end of file");
        return FLUID_FAILED;
    }

    FLUID_OK
}

/// Translation of `fluid_midi_file_read_varlen()`.
fn fluid_midi_file_read_varlen(mf: &mut FluidMidiFile<'_>) -> i32 {
    mf.varlen = 0;

    let mut i = 0;
    loop {
        if i == 4 {
            flog!(FLUID_ERR, "Invalid variable length number");
            return FLUID_FAILED;
        }

        let c = fluid_midi_file_getc(mf);

        if c < 0 {
            flog!(FLUID_ERR, "Unexpected end of file");
            return FLUID_FAILED;
        }

        if c & 0x80 != 0 {
            mf.varlen |= c & 0x7F;
            mf.varlen <<= 7;
        } else {
            mf.varlen += c;
            break;
        }
        i += 1;
    }

    FLUID_OK
}

/// Reads `len` bytes of event data into a new buffer of `len + 1` bytes,
/// as upstream's `FLUID_MALLOC(len + 1)` and `fluid_midi_file_read()`;
/// None (after the same failed read) when the data is not there or the
/// buffer can't be allocated.
fn read_event_data(mf: &mut FluidMidiFile<'_>, len: i32) -> Option<Vec<u8>> {
    // (data past the end of the buffer fails to read: don't allocate for it)
    if len > mf.buf_len - mf.buf_pos {
        fluid_midi_file_read(mf, None, len);
        return None;
    }

    let mut data = Vec::new();
    if data.try_reserve_exact(len as usize + 1).is_err() {
        flog!(FLUID_PANIC, "Out of memory");
        return None;
    }
    data.resize(len as usize + 1, 0);

    if fluid_midi_file_read(mf, Some(&mut data[..len as usize]), len) != FLUID_OK {
        return None;
    }

    Some(data)
}

/// Translation of `fluid_midi_file_read_event()`.
fn fluid_midi_file_read_event(mf: &mut FluidMidiFile<'_>, track: &mut FluidTrack) -> i32 {
    let mut status;
    let type_;
    let channel;
    let mut param1;
    let mut param2 = 0;

    /* read the delta-time of the event */
    if fluid_midi_file_read_varlen(mf) != FLUID_OK {
        flog!(
            FLUID_DBG,
            "Reading delta-time failed unexpectedly (track={})",
            track.num
        );
        return FLUID_FAILED;
    }

    mf.dtime = mf.dtime.wrapping_add(mf.varlen);

    /* read the status byte */
    status = fluid_midi_file_getc(mf);

    if status < 0 {
        flog!(FLUID_ERR, "Unexpected end of file");
        return FLUID_FAILED;
    }

    /* not a valid status byte: use the running status instead */
    if (status & 0x80) == 0 {
        if (mf.running_status & 0x80) == 0 {
            flog!(
                FLUID_ERR,
                "Undefined status and invalid running status (0x{:X}, 0x{:X})",
                status,
                mf.running_status
            );
            return FLUID_FAILED;
        }

        fluid_midi_file_push(mf, status);
        status = mf.running_status;
    }

    /* check what message we have */
    if status == MIDI_SYSEX {
        /* system exclusive */
        /* read the length of the message */
        if fluid_midi_file_read_varlen(mf) != FLUID_OK {
            flog!(
                FLUID_DBG,
                "Failed to read length of SYSEX msg (track={})",
                track.num
            );
            return FLUID_FAILED;
        }

        if mf.varlen != 0 {
            flog!(
                FLUID_DBG,
                "{}: {}: alloc metadata, len = {}",
                file!(),
                line!(),
                mf.varlen
            );
            /* read the data of the message */
            let Some(metadata) = read_event_data(mf, mf.varlen) else {
                flog!(
                    FLUID_DBG,
                    "Failed to read data of SYSEX msg (track={})",
                    track.num
                );
                return FLUID_FAILED;
            };

            let mut evt = new_fluid_midi_event();

            evt.dtime = mf.dtime as u32;
            let mut size = mf.varlen;

            if metadata[mf.varlen as usize - 1] as i32 == MIDI_EOX {
                size -= 1;
            }

            /* Add SYSEX event and indicate that its dynamically allocated and should be freed with event */
            let data: Box<[i8]> = metadata.iter().map(|&b| b as i8).collect();
            fluid_midi_event_set_sysex(&mut evt, data, size);
            fluid_track_add_event(track, evt);
            mf.dtime = 0;
        }

        FLUID_OK
    } else if status == MIDI_META_EVENT {
        /* meta events */
        let mut result = FLUID_OK;

        /* get the type of the meta message */
        type_ = fluid_midi_file_getc(mf);

        if type_ < 0 {
            flog!(FLUID_ERR, "Unexpected end of file");
            return FLUID_FAILED;
        }

        /* get the length of the data part */
        if fluid_midi_file_read_varlen(mf) != FLUID_OK {
            flog!(
                FLUID_DBG,
                "Failed to read length of META msg (track={})",
                track.num
            );
            return FLUID_FAILED;
        }

        if mf.varlen >= 255 {
            flog!(
                FLUID_DBG,
                "{}: {}: alloc metadata, len = {}",
                file!(),
                line!(),
                mf.varlen
            );
        }

        /* read the data */
        let metadata = if mf.varlen != 0 {
            match read_event_data(mf, mf.varlen) {
                Some(metadata) => metadata,
                None => {
                    flog!(FLUID_DBG, "Failed to read meta msg (track={})", track.num);
                    return FLUID_FAILED;
                }
            }
        } else {
            vec![0u8; 1]
        };
        let varlen = mf.varlen as usize;

        /* handle meta data */
        match type_ {
            MIDI_COPYRIGHT => {}

            MIDI_TRACK_NAME => {
                let name = &metadata[..varlen];
                let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(varlen)];
                fluid_track_set_name(track, Some(&String::from_utf8_lossy(name)));
            }

            MIDI_INST_NAME => {}

            MIDI_LYRIC | MIDI_TEXT => {
                let size = mf.varlen + 1;

                /* NULL terminate strings for safety */
                // (metadata[size - 1] is the zeroed extra byte)

                let mut evt = new_fluid_midi_event();

                evt.dtime = mf.dtime as u32;

                let tmp: Box<[i8]> = metadata[..size as usize].iter().map(|&b| b as i8).collect();

                fluid_midi_event_set_sysex_local(&mut evt, type_, tmp, size);
                fluid_track_add_event(track, evt);
                mf.dtime = 0;
            }

            MIDI_MARKER => {}

            MIDI_CUE_POINT => {} /* don't care much for text events */

            MIDI_EOT => {
                if mf.varlen != 0 {
                    flog!(FLUID_ERR, "Invalid length for EndOfTrack event");
                    result = FLUID_FAILED;
                } else {
                    mf.eot = true;
                    let mut evt = new_fluid_midi_event();

                    evt.dtime = mf.dtime as u32;
                    evt.type_ = MIDI_EOT as u8;
                    fluid_track_add_event(track, evt);
                    mf.dtime = 0;
                }
            }

            MIDI_SET_TEMPO => {
                if mf.varlen != 3 {
                    flog!(FLUID_ERR, "Invalid length for SetTempo meta event");
                    result = FLUID_FAILED;
                } else {
                    let tempo = ((metadata[0] as i32) << 16)
                        + ((metadata[1] as i32) << 8)
                        + metadata[2] as i32;
                    let mut evt = new_fluid_midi_event();

                    evt.dtime = mf.dtime as u32;
                    evt.type_ = MIDI_SET_TEMPO as u8;
                    evt.channel = 0;
                    evt.param1 = tempo as u32;
                    evt.param2 = 0;
                    fluid_track_add_event(track, evt);
                    mf.dtime = 0;
                }
            }

            MIDI_SMPTE_OFFSET => {
                if mf.varlen != 5 {
                    flog!(FLUID_ERR, "Invalid length for SMPTE Offset meta event");
                    result = FLUID_FAILED;
                }

                /* we don't use smtp */
            }

            MIDI_TIME_SIGNATURE => {
                if mf.varlen != 4 {
                    flog!(FLUID_ERR, "Invalid length for TimeSignature meta event");
                    result = FLUID_FAILED;
                } else {
                    let nominator = metadata[0] as i32;
                    // (upstream converts pow() to int, which is out of
                    // range from 2^31 on; only this debug message sees it)
                    let denominator = 2.0f64.powf(metadata[1] as f64) as i32;
                    let clocks = metadata[2] as i32;
                    let notes = metadata[3] as i32;

                    flog!(
                        FLUID_DBG,
                        "signature={}/{}, metronome={}, 32nd-notes={}",
                        nominator,
                        denominator,
                        clocks,
                        notes
                    );
                }
            }

            MIDI_KEY_SIGNATURE => {
                if mf.varlen != 2 {
                    flog!(FLUID_ERR, "Invalid length for KeySignature meta event");
                    result = FLUID_FAILED;
                }

                /* We don't care about key signatures anyway */
                /* sf = metadata[0];
                mi = metadata[1]; */
            }

            MIDI_SEQUENCER_EVENT => {}

            _ => {
                flog!(
                    FLUID_INFO,
                    "Ignoring unrecognized meta event type 0x{:X}",
                    type_
                );
            }
        }

        if mf.varlen >= 255 {
            flog!(FLUID_DBG, "{}: {}: free metadata", file!(), line!());
        }

        result
    } else {
        /* channel messages */
        // remember current status (not applicable to SYSEX and META events!)
        mf.running_status = status;
        type_ = status & 0xf0;
        channel = status & 0x0f;

        /* all channel message have at least 1 byte of associated data */
        param1 = fluid_midi_file_getc(mf);
        if param1 < 0 {
            flog!(FLUID_ERR, "Unexpected end of file");
            return FLUID_FAILED;
        }

        match type_ {
            NOTE_ON | NOTE_OFF | KEY_PRESSURE | CONTROL_CHANGE => {
                param2 = fluid_midi_file_getc(mf);
                if param2 < 0 {
                    flog!(FLUID_ERR, "Unexpected end of file");
                    return FLUID_FAILED;
                }
            }

            PROGRAM_CHANGE => {}

            CHANNEL_PRESSURE => {}

            PITCH_BEND => {
                param2 = fluid_midi_file_getc(mf);
                if param2 < 0 {
                    flog!(FLUID_ERR, "Unexpected end of file");
                    return FLUID_FAILED;
                }

                param1 = ((param2 & 0x7f) << 7) | (param1 & 0x7f);
                param2 = 0;
            }

            _ => {
                /* Can't possibly happen !? */
                flog!(FLUID_ERR, "Unrecognized MIDI event 0x{:X}", status);
                return FLUID_FAILED;
            }
        }

        let mut evt = new_fluid_midi_event();

        evt.dtime = mf.dtime as u32;
        evt.type_ = type_ as u8;
        evt.channel = channel as u8;
        evt.param1 = param1 as u32;
        evt.param2 = param2 as u32;
        fluid_track_add_event(track, evt);
        mf.dtime = 0;

        FLUID_OK
    }
}

/// Translation of `fluid_midi_file_get_division()`.
fn fluid_midi_file_get_division(midifile: &FluidMidiFile<'_>) -> i32 {
    midifile.division as i32
}

/******************************************************
 *
 *     fluid_track_t
 */

/// Translation of `new_fluid_midi_event()`: create a MIDI event
/// structure.
fn new_fluid_midi_event() -> FluidMidiEvent {
    FluidMidiEvent::default()
}

impl FluidMidiEvent {
    /// Translation of `new_fluid_midi_event()`: create a MIDI event
    /// structure.
    pub fn new() -> FluidMidiEvent {
        new_fluid_midi_event()
    }

    /// Translation of `fluid_midi_event_get_type()`: event type field
    /// (MIDI status byte without channel).
    pub fn get_type(&self) -> i32 {
        self.type_ as i32
    }

    /// Translation of `fluid_midi_event_set_type()`.
    pub fn set_type(&mut self, type_: i32) -> i32 {
        self.type_ = type_ as u8;
        FLUID_OK
    }

    /// Translation of `fluid_midi_event_get_channel()`.
    pub fn get_channel(&self) -> i32 {
        self.channel as i32
    }

    /// Translation of `fluid_midi_event_set_channel()`.
    pub fn set_channel(&mut self, chan: i32) -> i32 {
        self.channel = chan as u8;
        FLUID_OK
    }

    /// Translation of `fluid_midi_event_get_key()`: MIDI note number
    /// (0-127).
    pub fn get_key(&self) -> i32 {
        self.param1 as i32
    }

    /// Translation of `fluid_midi_event_set_key()`.
    pub fn set_key(&mut self, v: i32) -> i32 {
        self.param1 = v as u32;
        FLUID_OK
    }

    /// Translation of `fluid_midi_event_get_velocity()`: MIDI velocity
    /// number (0-127).
    pub fn get_velocity(&self) -> i32 {
        self.param2 as i32
    }

    /// Translation of `fluid_midi_event_set_velocity()`.
    pub fn set_velocity(&mut self, v: i32) -> i32 {
        self.param2 = v as u32;
        FLUID_OK
    }

    /// Translation of `fluid_midi_event_get_control()`: MIDI control
    /// number.
    pub fn get_control(&self) -> i32 {
        self.param1 as i32
    }

    /// Translation of `fluid_midi_event_set_control()`.
    pub fn set_control(&mut self, v: i32) -> i32 {
        self.param1 = v as u32;
        FLUID_OK
    }

    /// Translation of `fluid_midi_event_get_value()`: value field.
    pub fn get_value(&self) -> i32 {
        self.param2 as i32
    }

    /// Translation of `fluid_midi_event_set_value()`.
    pub fn set_value(&mut self, v: i32) -> i32 {
        self.param2 = v as u32;
        FLUID_OK
    }

    /// Translation of `fluid_midi_event_get_program()`: MIDI program
    /// number (0-127).
    pub fn get_program(&self) -> i32 {
        self.param1 as i32
    }

    /// Translation of `fluid_midi_event_set_program()`.
    pub fn set_program(&mut self, val: i32) -> i32 {
        self.param1 = val as u32;
        FLUID_OK
    }

    /// Translation of `fluid_midi_event_get_pitch()`: pitch value (14 bit
    /// value, 0-16383, 8192 is center).
    pub fn get_pitch(&self) -> i32 {
        self.param1 as i32
    }

    /// Translation of `fluid_midi_event_set_pitch()`.
    pub fn set_pitch(&mut self, val: i32) -> i32 {
        self.param1 = val as u32;
        FLUID_OK
    }

    /// Translation of `fluid_midi_event_set_sysex()`: assign SYSEX data to
    /// a MIDI event structure (the bytes between 0xF0 and 0xF7, which the
    /// event takes over).
    pub fn set_sysex(&mut self, data: &[u8]) -> i32 {
        let size = data.len() as i32;
        let data: Box<[i8]> = data.iter().map(|&b| b as i8).collect();
        fluid_midi_event_set_sysex(self, data, size);
        FLUID_OK
    }

    /// The `size` (param1) bytes of the event's SYSEX data.
    pub(crate) fn sysex_data(&self) -> &[i8] {
        match self.paramptr.as_deref() {
            Some(data) => &data[..(self.param1 as usize).min(data.len())],
            None => &[],
        }
    }
}

/// Translation of `fluid_midi_event_set_sysex()`.
fn fluid_midi_event_set_sysex(evt: &mut FluidMidiEvent, data: Box<[i8]>, size: i32) {
    fluid_midi_event_set_sysex_local(evt, MIDI_SYSEX, data, size);
}

/// Translation of `fluid_midi_event_set_sysex_LOCAL()` (always dynamic).
fn fluid_midi_event_set_sysex_local(
    evt: &mut FluidMidiEvent,
    type_: i32,
    data: Box<[i8]>,
    size: i32,
) {
    evt.type_ = type_ as u8;
    evt.paramptr = Some(data);
    evt.param1 = size as u32;
    evt.param2 = 1;
}

/// Translation of `new_fluid_track()`.
fn new_fluid_track(num: i32) -> FluidTrack {
    FluidTrack {
        name: None,
        num,
        events: Vec::new(),
        cur: 0,
        ticks: 0,
    }
}

/// Translation of `fluid_track_set_name()`.
fn fluid_track_set_name(track: &mut FluidTrack, name: Option<&str>) -> i32 {
    track.name = name.map(str::to_owned);
    FLUID_OK
}

/// Translation of `fluid_track_get_duration()`.
fn fluid_track_get_duration(track: &FluidTrack) -> i32 {
    let mut time: i32 = 0;

    for evt in &track.events {
        time = time.wrapping_add(evt.dtime as i32);
    }

    time
}

/// Translation of `fluid_track_add_event()`.
fn fluid_track_add_event(track: &mut FluidTrack, evt: FluidMidiEvent) -> i32 {
    // (the first event added is the current one, as cur is 0)
    track.events.push(evt);
    FLUID_OK
}

/// Translation of `fluid_track_next_event()`.
fn fluid_track_next_event(track: &mut FluidTrack) {
    if track.cur < track.events.len() {
        track.cur += 1;
    }
}

/// Translation of `fluid_track_reset()`.
fn fluid_track_reset(track: &mut FluidTrack) -> i32 {
    track.ticks = 0;
    track.cur = 0;
    FLUID_OK
}

/// Translation of `fluid_track_send_events()`.
fn fluid_track_send_events(
    track: &mut FluidTrack,
    synth: &mut FluidSynthInner,
    player: &mut FluidPlayerState,
    mut ticks: u32,
    seek_ticks: i32,
) {
    let seeking = seek_ticks >= 0;

    if seeking {
        ticks = seek_ticks as u32; /* update target ticks */

        // the track ticks is beyond the current ticks,
        // or if the track ticks is equal to the current ticks,
        // we want to play all events from the beginning of that tick
        if track.ticks >= ticks {
            fluid_track_reset(track); /* reset track if seeking backwards */
        }
    }

    loop {
        let Some(event) = track.events.get(track.cur) else {
            return;
        };

        if track.ticks.wrapping_add(event.dtime) > ticks {
            return;
        }

        track.ticks = track.ticks.wrapping_add(event.dtime);

        let type_ = event.type_ as i32;
        if type_ == MIDI_EOT {
            /* don't send EOT events to the callback */
        } else if seeking && track.ticks != ticks && (type_ == NOTE_ON || type_ == NOTE_OFF) {
            /* skip on/off messages */
        } else {
            // (the playback callback is fluid_synth_handle_midi_event())
            let chan_is_playing = event.channel as usize % MAX_NUMBER_OF_CHANNELS;
            synth.fluid_synth_handle_midi_event(event);
            if type_ == NOTE_ON && event.param2 != 0 && !player.channel_isplaying[chan_is_playing] {
                player.channel_isplaying[chan_is_playing] = true;
            }
        }

        if type_ == MIDI_SET_TEMPO {
            /* memorize the tempo change value coming from the MIDI file */
            player.miditempo = event.param1 as i32;
            fluid_player_update_tempo(player);
        }

        fluid_track_next_event(track);
    }
}

/******************************************************
 *
 *     fluid_player
 */

/// Translation of `check_for_on_notes()`.
fn check_for_on_notes(synth: &mut FluidSynthInner) -> bool {
    let mut res = false;
    for i in synth.fluid_synth_get_voicelist(1024, -1) {
        let vv = &synth.voice[i];
        if fluid_voice_is_on(vv) {
            res = true;
            flog!(
                FLUID_DBG,
                "Voice is on! channel {}, key {}",
                fluid_voice_get_channel(vv),
                fluid_voice_get_key(vv)
            );
        }
    }
    res
}

/// A MIDI file player (`fluid_player_t *`) that plays songs on a
/// [`Synth`] as it renders: the player advances with the audio written by
/// [`Synth::write_float`].
pub struct Player {
    synth: Synth,
    shared: Arc<FluidPlayerShared>,
    sample_timer: u64,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("status", &self.get_status())
            .finish_non_exhaustive()
    }
}

/// Translation of `new_fluid_player()`: create a new MIDI player.
fn new_fluid_player(synth: &Synth) -> Option<Player> {
    let mut inner = synth.lock();

    let mut state = FluidPlayerState {
        track: Vec::new(),
        loop_: 1,
        playlist: Vec::new(),
        currentfile: None,
        reset_synth_between_songs: false,
        start_ticks: 0,
        cur_ticks: 0,
        begin_msec: 0,
        start_msec: 0,
        cur_msec: 0,
        end_msec: -1,
        end_pedals_disabled: false,

        /* internal tempo (from MIDI file) in micro seconds per quarter note */
        sync_mode: 1, /* the player follows internal tempo change */
        miditempo: 500000,
        /* external tempo in micro seconds per quarter note */
        exttempo: 500000,
        /* tempo multiplier */
        multempo: 1.0f32,

        deltatime: 4.0,
        division: 0,
        channel_isplaying: [false; MAX_NUMBER_OF_CHANNELS],
    };
    // (player->last_callback_ticks belongs to the tick callback, which is
    // not translated; the playback callback is always
    // fluid_synth_handle_midi_event())

    let use_system_timer = inner.settings.str_equal("player.timing-source", "system");
    if use_system_timer {
        // (the system timer needs a thread, which this translation does
        // not have: fail as upstream does when it can't create the timer)
        flog!(
            FLUID_ERR,
            "The \"system\" player.timing-source is not supported"
        );
        return None;
    }

    let mut i = 0;
    inner.settings.getint("player.reset-synth", &mut i);
    /* fluid_player_handle_reset_synth() */
    state.reset_synth_between_songs = i != 0;

    let shared = Arc::new(FluidPlayerShared {
        status: AtomicI32::new(FLUID_PLAYER_READY),
        stopping: AtomicI32::new(0),
        seek_ticks: AtomicI32::new(-1),
        state: Mutex::new(state),
    });

    let weak: Weak<FluidPlayerShared> = Arc::downgrade(&shared);
    let sample_timer =
        inner.new_fluid_sample_timer(Box::new(move |synth: &mut FluidSynthInner, msec: u32| {
            match weak.upgrade() {
                Some(shared) => {
                    let mut state = lock_state(&shared);
                    fluid_player_callback(&shared, &mut state, synth, msec)
                }
                None => 0,
            }
        }));

    drop(inner);

    Some(Player {
        synth: synth.clone(),
        shared,
        sample_timer,
    })
}

/// The player's locked state.
fn lock_state(shared: &FluidPlayerShared) -> MutexGuard<'_, FluidPlayerState> {
    shared.state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `delete_fluid_player()`.
impl Drop for Player {
    fn drop(&mut self) {
        let mut synth = self.synth.lock();
        {
            let mut state = lock_state(&self.shared);
            fluid_player_stop(&self.shared, &mut state);
            fluid_player_reset(&mut state);
        }

        synth.delete_fluid_sample_timer(self.sample_timer);

        // (the playlist is freed with the state)
    }
}

/// Translation of `fluid_player_settings()`.
pub(crate) fn fluid_player_settings(settings: &mut SettingsInner) {
    /* player.timing-source can be either "system" (use system timer)
    or "sample" (use timer based on number of written samples) */
    settings.register_str("player.timing-source", Some("sample"), 0);
    settings.add_option("player.timing-source", "sample");
    settings.add_option("player.timing-source", "system");

    /* Selects whether the player should reset the synth between songs, or not. */
    settings.register_int("player.reset-synth", 1, 0, 1, FLUID_HINT_TOGGLED);
}

/// Translation of `fluid_player_reset()`.
fn fluid_player_reset(player: &mut FluidPlayerState) -> i32 {
    player.track.clear();

    for i in 0..MAX_NUMBER_OF_CHANNELS {
        player.channel_isplaying[i] = false;
    }

    /*    player->current_file = NULL; */
    /*    player->status = FLUID_PLAYER_READY; */
    /*    player->loop = 1; */
    player.division = 0;
    player.miditempo = 500000;
    player.deltatime = 4.0;
    0
}

/// Translation of `fluid_player_add_track()`.
fn fluid_player_add_track(player: &mut FluidPlayerState, track: FluidTrack) -> i32 {
    if player.track.len() < MAX_NUMBER_OF_TRACKS {
        player.track.push(track);
        FLUID_OK
    } else {
        FLUID_FAILED
    }
}

/// Translation of `fluid_player_load()` (of a song in memory).
fn fluid_player_load(player: &mut FluidPlayerState, item: usize) -> i32 {
    /* This file is specified by a pre-loaded buffer; load from memory */
    // (taken out of the playlist while the player loads it)
    let buffer = std::mem::take(&mut player.playlist[item].buffer);
    flog!(
        FLUID_DBG,
        "{}: {}: Loading midifile from memory ({:p})",
        file!(),
        line!(),
        buffer.as_ptr()
    );

    let result = fluid_player_load_buffer(player, &buffer);
    player.playlist[item].buffer = buffer;
    result
}

fn fluid_player_load_buffer(player: &mut FluidPlayerState, buffer: &[u8]) -> i32 {
    let Some(mut midifile) = new_fluid_midi_file(buffer) else {
        return FLUID_FAILED;
    };

    player.division = fluid_midi_file_get_division(&midifile) as u32;
    fluid_player_update_tempo(player); // Update deltatime
                                       /*FLUID_LOG(FLUID_DBG, "quarter note division=%d\n", player->division); */

    if fluid_midi_file_load_tracks(&mut midifile, player) != FLUID_OK {
        return FLUID_FAILED;
    }

    FLUID_OK
}

/// Translation of `fluid_player_advancefile()`.
fn fluid_player_advancefile(player: &mut FluidPlayerState) {
    if player.playlist.is_empty() {
        return; /* No files to play */
    }

    if let Some(current) = player.currentfile {
        player.currentfile = if current + 1 < player.playlist.len() {
            Some(current + 1)
        } else {
            None
        };
    }

    if player.currentfile.is_none() {
        if player.loop_ == 0 {
            return; /* We're done playing */
        }

        if player.loop_ > 0 {
            player.loop_ -= 1;
        }

        player.currentfile = Some(0);
    }
}

/// Translation of `fluid_player_playlist_load()`.
fn fluid_player_playlist_load(
    shared: &FluidPlayerShared,
    player: &mut FluidPlayerState,
    msec: u32,
) {
    loop {
        fluid_player_advancefile(player);

        let Some(current_playitem) = player.currentfile else {
            /* Failed to find next song, probably since we're finished */
            shared.status.store(FLUID_PLAYER_DONE, Ordering::SeqCst);
            return;
        };

        fluid_player_reset(player);

        if fluid_player_load(player, current_playitem) == FLUID_OK {
            break;
        }
    }

    /* Successfully loaded midi file */

    player.begin_msec = msec as i32;
    player.start_msec = msec as i32;
    player.start_ticks = 0;
    player.cur_ticks = 0;

    for track in player.track.iter_mut() {
        fluid_track_reset(track);
    }
}

/// Translation of `fluid_player_callback()`.
fn fluid_player_callback(
    shared: &FluidPlayerShared,
    player: &mut FluidPlayerState,
    synth: &mut FluidSynthInner,
    msec: u32,
) -> i32 {
    let mut loadnextfile;
    let mut status = FLUID_PLAYER_DONE;
    let mut mute_event = new_fluid_midi_event();

    loadnextfile = player.currentfile.is_none();

    mute_event.set_type(CONTROL_CHANGE);
    mute_event.param1 = ALL_SOUND_OFF as u32;
    mute_event.param2 = 1;

    if fluid_player_get_status(shared) != FLUID_PLAYER_PLAYING {
        if shared.stopping.load(Ordering::SeqCst) != 0 {
            for i in 0..MAX_NUMBER_OF_CHANNELS {
                if player.channel_isplaying[i] {
                    mute_event.set_channel(i as i32);
                    synth.fluid_synth_handle_midi_event(&mute_event);
                    player.channel_isplaying[i] = false;
                }
            }
            shared.stopping.store(0, Ordering::SeqCst);
        }
        return 1;
    }
    loop {
        if loadnextfile {
            loadnextfile = false;
            fluid_player_playlist_load(shared, player, msec);

            if player.currentfile.is_none() {
                return 0;
            }
        }

        if msec < player.cur_msec {
            // overflow of fluid_synth_get_ticks()
            flog!(
                FLUID_ERR,
                "The maximum playback duration has been reached. Terminating player!"
            );
            fluid_player_stop(shared, player);
            fluid_player_seek(shared, player, 0);
            player.cur_ticks = 0;
            return 0;
        }

        player.cur_msec = msec;
        let deltatime = player.deltatime;
        // FIXME (upstream): the conversion to int is undefined when the
        // ticks are out of its range (a fast tempo and a fine division);
        // this converts as x86-64 does.
        player.cur_ticks = player.start_ticks.wrapping_add(c_double_to_int(
            (player.cur_msec.wrapping_sub(player.start_msec as u32)) as f64 / deltatime as f64
                + 0.5,
        )); /* 0.5 to average overall error when casting */

        let seek_ticks = shared.seek_ticks.load(Ordering::SeqCst);
        if seek_ticks >= 0 {
            for i in 0..MAX_NUMBER_OF_CHANNELS {
                if player.channel_isplaying[i] {
                    mute_event.set_channel(i as i32);
                    synth.fluid_synth_handle_midi_event(&mute_event);
                    player.channel_isplaying[i] = false;
                }
            }
        }

        // (the tracks are taken out of the player while they send events)
        let mut tracks = std::mem::take(&mut player.track);
        for track in tracks.iter_mut() {
            let cur_ticks = player.cur_ticks as u32;
            fluid_track_send_events(track, synth, player, cur_ticks, seek_ticks);
            if !track.eot() {
                status = FLUID_PLAYER_PLAYING;
            }
        }
        player.track = tracks;

        if seek_ticks >= 0 {
            player.start_ticks = seek_ticks; /* tick position of last tempo value (which is now) */
            player.cur_ticks = seek_ticks;
            player.begin_msec = msec as i32; /* only used to calculate the duration of playing */
            player.start_msec = msec as i32; /* should be the (synth)-time of the last tempo change */
            shared.seek_ticks.store(-1, Ordering::SeqCst); /* clear seek_ticks */
        }

        let is_last = matches!(player.currentfile, Some(c) if c + 1 >= player.playlist.len());
        if is_last && player.loop_ == 0 {
            /* Once we've run out of MIDI events, keep playing until no voices are active */
            if status == FLUID_PLAYER_DONE && synth.fluid_synth_get_active_voice_count() > 0 {
                /* The first time we notice we've run out of MIDI events but there are still active voices, disable all hold pedals */
                if !player.end_pedals_disabled {
                    if check_for_on_notes(synth) {
                        flog!(FLUID_WARN, "End of the MIDI file reached, but not all notes have received a note off event! OFFing them now! Run with --verbose to spot pending voices.");
                    }

                    for i in 0..synth.midi_channels {
                        synth.fluid_synth_cc(i, SUSTAIN_SWITCH, 0);
                        synth.fluid_synth_cc(i, SOSTENUTO_SWITCH, 0);
                        synth.fluid_synth_cc(i, ALL_NOTES_OFF, 0);
                    }

                    player.end_pedals_disabled = true;
                }

                status = FLUID_PLAYER_PLAYING;
            }

            /* Once no voices are active, if end_msec hasn't been scheduled, schedule it so we wait for reverb, etc to finish */
            if status == FLUID_PLAYER_DONE && player.end_msec < 0 {
                player.end_msec = msec.wrapping_add(FLUID_PLAYER_STOP_GRACE_MS) as i32;
            }
            /* If end_msec has been scheduled and is in the future, keep playing */
            if player.end_msec >= 0 && msec < player.end_msec as u32 {
                status = FLUID_PLAYER_PLAYING;
            }
        }

        /* Once there's no reason to keep playing, we're actually done */
        if status == FLUID_PLAYER_DONE {
            flog!(
                FLUID_DBG,
                "{}: {}: Duration={:.3} sec",
                file!(),
                line!(),
                msec.wrapping_sub(player.begin_msec as u32) as f64 / 1000.0
            );

            if player.reset_synth_between_songs {
                synth.fluid_synth_system_reset();
            }

            loadnextfile = true;
        }

        // (there is no tick callback)

        if !loadnextfile {
            break;
        }
    }

    /* do not update the status if the player has been stopped already */
    let _ = shared.status.compare_exchange(
        FLUID_PLAYER_PLAYING,
        status,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );

    1
}

/// Translation of `fluid_player_play()`.
fn fluid_player_play(
    shared: &FluidPlayerShared,
    player: &mut FluidPlayerState,
    synth: &mut FluidSynthInner,
    timer: u64,
) -> i32 {
    if fluid_player_get_status(shared) == FLUID_PLAYER_PLAYING || player.playlist.is_empty() {
        return FLUID_OK;
    }

    /* !player->use_system_timer */
    synth.fluid_sample_timer_reset(timer);
    player.cur_msec = 0;

    /* If we're at the end of the playlist and there are no loops left, loop once */
    if player.currentfile.is_none() && player.loop_ == 0 {
        player.loop_ = 1;
    }

    player.end_msec = -1;
    player.end_pedals_disabled = false;

    shared.status.store(FLUID_PLAYER_PLAYING, Ordering::SeqCst);

    FLUID_OK
}

/// Translation of `fluid_player_stop()`.
fn fluid_player_stop(shared: &FluidPlayerShared, player: &mut FluidPlayerState) -> i32 {
    shared.status.store(FLUID_PLAYER_DONE, Ordering::SeqCst);
    shared.stopping.store(1, Ordering::SeqCst);
    let tick = fluid_player_get_current_tick(player);
    fluid_player_seek(shared, player, tick);
    FLUID_OK
}

/// Translation of `fluid_player_get_status()`.
fn fluid_player_get_status(shared: &FluidPlayerShared) -> i32 {
    shared.status.load(Ordering::SeqCst)
}

/// Translation of `fluid_player_seek()`.
fn fluid_player_seek(shared: &FluidPlayerShared, player: &FluidPlayerState, ticks: i32) -> i32 {
    if ticks < 0
        || (fluid_player_get_status(shared) != FLUID_PLAYER_READY
            && ticks > fluid_player_get_total_ticks(player))
    {
        return FLUID_FAILED;
    }

    if fluid_player_get_status(shared) == FLUID_PLAYER_PLAYING {
        if shared
            .seek_ticks
            .compare_exchange(-1, ticks, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            // new seek position has been set, as no previous seek was in progress
            return FLUID_OK;
        }
    } else {
        // If the player is not currently playing, a new seek position can be set at any time. This allows
        // the user to do:
        // fluid_player_stop();
        // fluid_player_seek(0); // to beginning
        shared.seek_ticks.store(ticks, Ordering::SeqCst);
        return FLUID_OK;
    }

    // a previous seek is still in progress or hasn't been processed yet
    FLUID_FAILED
}

/// Translation of `fluid_player_update_tempo()`: update the MIDI player
/// internal deltatime dependent of actual tempo.
fn fluid_player_update_tempo(player: &mut FluidPlayerState) {
    let tempo: i32; /* tempo in micro seconds by quarter note */
    let mut deltatime: f32;

    /* do nothing if the division is still unknown to avoid a div by zero */
    if player.division == 0 {
        return;
    }

    if player.sync_mode != 0 {
        /* take internal tempo from MIDI file */
        tempo = player.miditempo;
        /* compute deltattime (in ms) from current tempo and apply tempo multiplier */
        deltatime = tempo as f32 / player.division as f32 / 1000.0f32;
        deltatime /= player.multempo; /* multiply tempo */
    } else {
        /* take  external tempo */
        tempo = player.exttempo;
        /* compute deltattime (in ms) from current tempo */
        deltatime = tempo as f32 / player.division as f32 / 1000.0f32;
    }

    player.deltatime = deltatime;

    player.start_msec = player.cur_msec as i32;
    player.start_ticks = player.cur_ticks;

    flog!(
        FLUID_DBG,
        "tempo={}, tick time={:.6} msec, cur time={} msec, cur tick={}",
        tempo,
        player.deltatime as f64,
        player.cur_msec as i32,
        player.cur_ticks
    );
}

/// Translation of `fluid_player_get_current_tick()`.
fn fluid_player_get_current_tick(player: &FluidPlayerState) -> i32 {
    player.cur_ticks
}

/// Translation of `fluid_player_get_total_ticks()`: looks through all
/// available MIDI tracks and gets the absolute tick of the very last event
/// to play.
fn fluid_player_get_total_ticks(player: &FluidPlayerState) -> i32 {
    let mut max_ticks = 0;

    for track in &player.track {
        let ticks = fluid_track_get_duration(track);

        if ticks > max_ticks {
            max_ticks = ticks;
        }
    }

    max_ticks
}

impl Player {
    /// Translation of `new_fluid_player()`: create a new MIDI player for
    /// a synth (None if the synth's settings ask for the "system"
    /// `player.timing-source`, which is not supported).
    pub fn new(synth: &Synth) -> Option<Player> {
        new_fluid_player(synth)
    }

    /// Translation of `fluid_player_add_mem()`: add a MIDI file to the
    /// player queue, from a buffer in memory (which is copied). Returns
    /// #FLUID_OK or #FLUID_FAILED.
    pub fn add_mem(&self, buffer: &[u8]) -> i32 {
        /* Take a copy of the buffer, so the caller can free immediately. */
        let mut buf_copy = Vec::new();
        if buf_copy.try_reserve_exact(buffer.len()).is_err() {
            flog!(FLUID_PANIC, "Out of memory");
            return FLUID_FAILED;
        }

        buf_copy.extend_from_slice(buffer);
        lock_state(&self.shared)
            .playlist
            .push(FluidPlaylistItem { buffer: buf_copy });
        FLUID_OK
    }

    /// Translation of `fluid_player_play()`: activates play mode for a
    /// MIDI player if not already playing.
    ///
    /// If the list of files added to the player has completed its
    /// requested number of loops, the playlist will be restarted from the
    /// beginning with a loop count of 1.
    pub fn play(&self) -> i32 {
        let mut synth = self.synth.lock();
        let mut state = lock_state(&self.shared);
        fluid_player_play(&self.shared, &mut state, &mut synth, self.sample_timer)
    }

    /// Translation of `fluid_player_stop()`: pauses the MIDI playback.
    ///
    /// It will not rewind to the beginning of the file, use
    /// [`Player::seek`] for this purpose.
    pub fn stop(&self) -> i32 {
        let mut state = lock_state(&self.shared);
        fluid_player_stop(&self.shared, &mut state)
    }

    /// Translation of `fluid_player_get_status()`: the player status
    /// (`FLUID_PLAYER_READY`, `FLUID_PLAYER_PLAYING`, ...).
    pub fn get_status(&self) -> i32 {
        fluid_player_get_status(&self.shared)
    }

    /// Translation of `fluid_player_seek()`: seek in the currently playing
    /// file (to a tick, which takes effect when the synth next calls the
    /// player). Fails if ticks is negative or after the latest tick of the
    /// file, or while playing if another seek is in progress.
    pub fn seek(&self, ticks: i32) -> i32 {
        let state = lock_state(&self.shared);
        fluid_player_seek(&self.shared, &state, ticks)
    }

    /// Translation of `fluid_player_set_loop()`: times left to loop the
    /// playlist (-1 means loop infinitely).
    pub fn set_loop(&self, loop_: i32) -> i32 {
        lock_state(&self.shared).loop_ = loop_;
        FLUID_OK
    }

    /// Translation of `fluid_player_get_current_tick()`.
    pub fn get_current_tick(&self) -> i32 {
        fluid_player_get_current_tick(&lock_state(&self.shared))
    }

    /// Translation of `fluid_player_get_total_ticks()`: the absolute tick
    /// of the very last event to play.
    pub fn get_total_ticks(&self) -> i32 {
        fluid_player_get_total_ticks(&lock_state(&self.shared))
    }
}
