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
// fluid_synth.c and fluid_synth.h in SDL_mixer 3.3.0's FluidSynth 2.5.7
// (see NOTICE) that SDL_mixer reaches: creating the synth, the MIDI
// channel messages, SYSEX (tuning, GM, GS and XG), voice allocation,
// SoundFont loading, the reverb and chorus setup and rendering with
// fluid_synth_write_float(). The synth's recursive API mutex is the
// `Mutex` of the `Synth` handle; functions that upstream calls through
// the public API from inside the synth (as the MIDI player's sample timer
// does) are methods of the locked `FluidSynthInner`, which count the API
// nesting the same way. Settings changed after the synth has been
// created do not reach it (upstream's realtime settings callbacks; the
// settings SDL_mixer sets are all set before), and SoundFont unloading,
// sample rate and polyphony changes, LADSPA, the sequencer and the
// remaining getters and setters of the public API are left out.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::fluidsynth_h::*;
use crate::fluidsynth_priv::{fluid_clip, params, FLUID_BUFSIZE, FLUID_MIXER_MAX_BUFFERS_DEFAULT};
use crate::midi::midi::*;
use crate::rvoice::chorus::*;
use crate::rvoice::iir_filter::{fluid_iir_filter_init_table, FluidIirSincos, SINCOS_TAB_SIZE};
use crate::rvoice::rev::*;
use crate::rvoice::rvoice_event::{FluidRvoiceEventhandler, RvoiceFunction, RvoiceObject};
use crate::rvoice::rvoice_mixer::fluid_rvoice_mixer_render;
use crate::sfloader::defsfont::FluidZoneRange;
use crate::sfloader::sfont::{FluidPreset, FluidSfont, Sample, SfLoader};
use crate::synth::chan::*;
use crate::synth::gen::*;
use crate::synth::modulator::*;
use crate::synth::tuning::FluidTuning;
use crate::synth::voice::*;
use crate::utils::conv::{fluid_hz2ct, fluid_sec2tc};
use crate::utils::conv_tables::FLUID_PEAK_ATTENUATION;
use crate::utils::settings::Settings;
use crate::utils::sys::{
    flog, fluid_curtime, fluid_log, FLUID_DBG, FLUID_ERR, FLUID_INFO, FLUID_WARN,
};
use crate::FluidReal;

/***************************************************************
 *
 *                         DEFINES
 */
#[allow(dead_code)]
pub(crate) const FLUID_NUM_PROGRAMS: i32 = 128;
pub(crate) const DRUM_INST_BANK: i32 = 128;

pub(crate) const FLUID_UNSET_PROGRAM: i32 = 128; /* Program number used to unset a preset */

pub(crate) const FLUID_REVERB_DEFAULT_DAMP: f32 = 0.3f32; /* Default reverb damping */
pub(crate) const FLUID_REVERB_DEFAULT_LEVEL: f32 = 0.7f32; /* Default reverb level */
pub(crate) const FLUID_REVERB_DEFAULT_ROOMSIZE: f32 = 0.5f32; /* Default reverb room size */
pub(crate) const FLUID_REVERB_DEFAULT_WIDTH: f32 = 0.8f32; /* Default reverb width */

pub(crate) const FLUID_CHORUS_DEFAULT_DEPTH: f32 = 4.25f32; /* Default chorus depth */
pub(crate) const FLUID_CHORUS_DEFAULT_LEVEL: f32 = 0.6f32; /* Default chorus level */
pub(crate) const FLUID_CHORUS_DEFAULT_N: i32 = 3; /* Default chorus voice count */
pub(crate) const FLUID_CHORUS_DEFAULT_SPEED: f32 = 0.2f32; /* Default chorus speed */
pub(crate) const FLUID_CHORUS_DEFAULT_TYPE: i32 = FLUID_CHORUS_MOD_SINE; /* Default chorus waveform type */

/* enum fluid_midi_bank_select: Bank Select MIDI message styles. Default
 * style is GS. */
pub(crate) const FLUID_BANK_STYLE_GM: i32 = 0; /* GM style, bank = 0 always (CC0/MSB and CC32/LSB ignored) */
pub(crate) const FLUID_BANK_STYLE_GS: i32 = 1; /* GS style, bank = CC0/MSB (CC32/LSB ignored) */
pub(crate) const FLUID_BANK_STYLE_XG: i32 = 2; /* XG style, bank = CC32/LSB (CC0/MSB ignored) */
pub(crate) const FLUID_BANK_STYLE_MMA: i32 = 3; /* MMA style bank = 128*MSB+LSB */

/* enum fluid_synth_status */
#[allow(dead_code)]
pub(crate) const FLUID_SYNTH_CLEAN: i32 = 0;
pub(crate) const FLUID_SYNTH_PLAYING: i32 = 1;
#[allow(dead_code)]
pub(crate) const FLUID_SYNTH_QUIET: i32 = 2;
#[allow(dead_code)]
pub(crate) const FLUID_SYNTH_STOPPED: i32 = 3;

/* enum fluid_msgs_note_cut */
pub(crate) const FLUID_MSGS_DISABLED: i32 = 0;
pub(crate) const FLUID_MSGS_DRUM_CUT: i32 = 1;
#[allow(dead_code)]
pub(crate) const FLUID_MSGS_ALL_CUT: i32 = 2;

pub(crate) const SYNTH_REVERB_CHANNEL: usize = 0;
pub(crate) const SYNTH_CHORUS_CHANNEL: usize = 1;

/// Translation of `fluid_timer_callback_t` for sample timers: called with
/// the synth and the milliseconds since the timer's start; returning 0
/// finishes the timer.
pub(crate) type FluidSampleTimerCallback = Box<dyn FnMut(&mut FluidSynthInner, u32) -> i32 + Send>;

/// `unsigned long` (the sample timers' start tick).
#[cfg(windows)]
type CULong = u32;
#[cfg(not(windows))]
type CULong = u64;

/***************************************************************
 *                    FLUID SAMPLE TIMERS
 *    Timers that use written audio data as timing reference
 */
/// Translation of `fluid_sample_timer_t` (with an ID in place of its
/// pointer).
pub(crate) struct FluidSampleTimer {
    pub id: u64,
    starttick: CULong,
    callback: Option<FluidSampleTimerCallback>,
    isfinished: bool,
}

/// A bank of the synth's tunings (128 programs).
pub(crate) type TuningBank = Vec<Option<Arc<FluidTuning>>>;

/// Translation of `fluid_synth_t`.
///
/// Mutual exclusion notes (as of 1.1.2):
///
/// All variables are considered belongning to the "public API" thread,
/// which processes all MIDI, except for:
///
/// ticks_since_start - atomic, set by rendering thread only
/// cpu_load - atomic, set by rendering thread only
/// cur, curmax, dither_index - used by rendering thread only
/// ladspa_fx - same instance copied in rendering thread. Synchronising handled internally.
pub(crate) struct FluidSynthInner {
    pub(crate) public_api_count: i32, /* How many times the mutex is currently locked */

    pub(crate) settings: Settings, /* the synthesizer settings */
    pub(crate) device_id: i32,     /* Device ID used for SYSEX messages */
    pub(crate) polyphony: i32,     /* Maximum polyphony */
    pub(crate) with_reverb: i32,   /* Should the synth use the built-in reverb unit? */
    pub(crate) with_chorus: i32,   /* Should the synth use the built-in chorus unit? */
    pub(crate) verbose: i32,       /* Turn verbose mode on? */
    pub(crate) sample_rate: f64,   /* The sample rate */
    pub(crate) midi_channels: i32, /* the number of MIDI channels (>= 16) */
    pub(crate) bank_select: i32,   /* the style of Bank Select MIDI messages */
    #[allow(dead_code)]
    pub(crate) audio_channels: i32, /* the number of audio channels (1 channel=left+right) */
    pub(crate) audio_groups: i32, /* the number of (stereo) 'sub'groups from the synth. Typically equal to audio_channels. */
    pub(crate) effects_channels: i32, /* the number of effects channels (>= 2) */
    pub(crate) effects_groups: i32, /* the number of effects units (>= 1) */
    pub(crate) state: i32,        /* the synthesizer state */
    pub(crate) ticks_since_start: u32, /* the number of audio samples since the start */
    pub(crate) start: u32,        /* the start in msec, as returned by system clock */
    pub(crate) overflow: FluidOverflowPrio, /* parameters for overflow priority (aka voice-stealing) */

    pub(crate) loaders: Vec<SfLoader>, /* the SoundFont loaders */
    pub(crate) sfont: Vec<FluidSfont>, /* List of fluid_sfont_info_t for each loaded SoundFont (remains until SoundFont is unloaded) */
    pub(crate) sfont_id: i32,          /* Incrementing ID assigned to each loaded SoundFont */

    pub(crate) gain: f32,                  /* master gain */
    pub(crate) channel: Vec<FluidChannel>, /* the channels */
    pub(crate) nvoice: i32, /* the length of the synthesis process array (max polyphony allowed) */
    pub(crate) voice: Vec<FluidVoice>, /* the synthesis voices */
    pub(crate) active_voice_count: i32, /* count of active voices */
    pub(crate) noteid: u32, /* the id is incremented for every new note. it's used for noteoff's  */
    pub(crate) storeid: u32,
    pub(crate) fromkey_portamento: i32, /* fromkey portamento */
    pub(crate) eventhandler: FluidRvoiceEventhandler,

    /* Shadow of reverb parameter: roomsize, damping, width, level */
    pub(crate) reverb_param: [f64; FLUID_REVERB_PARAM_LAST],

    /* Shadow of chorus parameter: chorus number, level, speed, depth, type */
    pub(crate) chorus_param: [f64; FLUID_CHORUS_PARAM_LAST],

    pub(crate) cur: i32, /* the current sample in the audio buffers to be output */
    pub(crate) curmax: i32, /* current amount of samples present in the audio buffers */
    pub(crate) dither_index: i32, /* current index in random dither value buffer: fluid_synth_(write_s16|dither_s16) */

    pub(crate) cpu_load: f32, /* CPU load in percent (CPU time required / audio synthesized time * 100) */

    pub(crate) tuning: Option<Vec<Option<TuningBank>>>, /* 128 banks of 128 programs for the tunings */

    pub(crate) sample_timers: Vec<FluidSampleTimer>, /* List of timers triggered before a block is processed */
    pub(crate) next_sample_timer_id: u64,
    pub(crate) min_note_length_ticks: u32, /* If note-offs are triggered just after a note-on, they will be delayed */

    #[allow(dead_code)]
    pub(crate) cores: i32, /* Number of CPU cores (1 by default) */

    pub(crate) default_mod: Vec<FluidMod>, /* the (dynamic) list of default modulators */

    #[allow(dead_code)]
    pub(crate) custom_filter_type: i32, /* filter type of the user-defined filter currently used for all voices */
    #[allow(dead_code)]
    pub(crate) custom_filter_flags: i32, /* filter type of the user-defined filter currently used for all voices */
    pub(crate) msgs_note_cut_mode: i32,

    /* Portamento time mode settings */
    pub(crate) portamento_time_mode: i32, /* Global portamento time mode */
    pub(crate) portamento_time_has_seen_lsb: bool, /* Flag to track if LSB has been seen (for auto mode) */

    pub(crate) iir_sincos_table: Arc<[FluidIirSincos]>, /* Table of sin/cos values for IIR filter */
}

impl std::fmt::Debug for FluidSynthInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FluidSynthInner")
            .field("sample_rate", &self.sample_rate)
            .field("polyphony", &self.polyphony)
            .field("active_voice_count", &self.active_voice_count)
            .finish_non_exhaustive()
    }
}

/// A FluidSynth instance (`fluid_synth_t *`): a handle to the shared,
/// locked synth, which a MIDI [`Player`](crate::Player) drives.
#[derive(Clone, Debug)]
pub struct Synth {
    pub(crate) inner: Arc<Mutex<FluidSynthInner>>,
}

/***************************************************************
 *
 *                         GLOBAL
 */

/// A modulator, as `fluid_mod_set_source1()`, `fluid_mod_set_source2()`,
/// `fluid_mod_set_dest()` and `fluid_mod_set_amount()` program it in
/// `fluid_synth_init()`.
const fn def_mod(src1: u8, flags1: u8, src2: u8, flags2: u8, dest: usize, amount: f64) -> FluidMod {
    FluidMod {
        dest: dest as u8,
        src1,
        flags1,
        src2,
        flags2,
        trans: 0,
        amount,
    }
}

/* default modulators
 * SF2.01 page 52 ff:
 *
 * There is a set of predefined default modulators. They have to be
 * explicitly overridden by the sound font in order to turn them off.
 */

/* SF2.01 page 53 section 8.4.1: MIDI Note-On Velocity to Initial Attenuation */
pub(crate) static DEFAULT_VEL2ATT_MOD: FluidMod = def_mod(
    FLUID_MOD_VELOCITY, /* Source. VELOCITY corresponds to 'index=2'. */
    FLUID_MOD_GC        /* Not a MIDI continuous controller */
        | FLUID_MOD_CONCAVE  /* Curve shape. Corresponds to 'type=1' */
        | FLUID_MOD_UNIPOLAR /* Polarity. Corresponds to 'P=0' */
        | FLUID_MOD_NEGATIVE, /* Direction. Corresponds to 'D=1' */
    0,
    0,                             /* No 2nd source */
    GEN_ATTENUATION,               /* Target: Initial attenuation */
    FLUID_PEAK_ATTENUATION as f64, /* Modulation amount: 960 */
);

/* SF2.01 page 53 section 8.4.2: MIDI Note-On Velocity to Filter Cutoff
 * Have to make a design decision here. The specs don't make any sense this way or another.
 * One sound font, 'Kingston Piano', which has been praised for its quality, tries to
 * override this modulator with an amount of 0 and positive polarity (instead of what
 * the specs say, D=1) for the secondary source.
 * So if we change the polarity to 'positive', one of the best free sound fonts works...
 */
pub(crate) static DEFAULT_VEL2FILTER_MOD: FluidMod = def_mod(
    FLUID_MOD_VELOCITY, /* Index=2 */
    FLUID_MOD_GC        /* CC=0 */
        | FLUID_MOD_LINEAR   /* type=0 */
        | FLUID_MOD_UNIPOLAR /* P=0 */
        | FLUID_MOD_NEGATIVE, /* D=1 */
    FLUID_MOD_VELOCITY, /* Index=2 */
    FLUID_MOD_GC        /* CC=0 */
        | FLUID_MOD_SWITCH   /* type=3 */
        | FLUID_MOD_UNIPOLAR /* P=0 */
        // do not remove       | FLUID_MOD_NEGATIVE                         /* D=1 */
        | FLUID_MOD_POSITIVE, /* D=0 */
    GEN_FILTERFC,       /* Target: Initial filter cutoff */
    -2400.0,
);

/* SF2.01 page 53 section 8.4.3: MIDI Channel pressure to Vibrato LFO pitch depth */
pub(crate) static DEFAULT_AT2VIBLFO_MOD: FluidMod = def_mod(
    FLUID_MOD_CHANNELPRESSURE, /* Index=13 */
    FLUID_MOD_GC               /* CC=0 */
        | FLUID_MOD_LINEAR   /* type=0 */
        | FLUID_MOD_UNIPOLAR /* P=0 */
        | FLUID_MOD_POSITIVE, /* D=0 */
    0,
    0,                 /* no second source */
    GEN_VIBLFOTOPITCH, /* Target: Vib. LFO => pitch */
    50.0,
);

/* SF2.01 page 53 section 8.4.4: Mod wheel (Controller 1) to Vibrato LFO pitch depth */
pub(crate) static DEFAULT_MOD2VIBLFO_MOD: FluidMod = def_mod(
    MODULATION_MSB as u8, /* Index=1 */
    FLUID_MOD_CC          /* CC=1 */
        | FLUID_MOD_LINEAR   /* type=0 */
        | FLUID_MOD_UNIPOLAR /* P=0 */
        | FLUID_MOD_POSITIVE, /* D=0 */
    0,
    0,                 /* no second source */
    GEN_VIBLFOTOPITCH, /* Target: Vib. LFO => pitch */
    50.0,
);

/* SF2.01 page 55 section 8.4.5: MIDI continuous controller 7 to initial attenuation*/
pub(crate) static DEFAULT_ATT_MOD: FluidMod = def_mod(
    VOLUME_MSB as u8, /* index=7 */
    FLUID_MOD_CC      /* CC=1 */
        | FLUID_MOD_CONCAVE  /* type=1 */
        | FLUID_MOD_UNIPOLAR /* P=0 */
        | FLUID_MOD_NEGATIVE, /* D=1 */
    0,
    0,                             /* No second source */
    GEN_ATTENUATION,               /* Target: Initial attenuation */
    FLUID_PEAK_ATTENUATION as f64, /* Amount: 960 */
);

/* SF2.01 page 55 section 8.4.6 MIDI continuous controller 10 to Pan Position */
pub(crate) static DEFAULT_PAN_MOD: FluidMod = def_mod(
    PAN_MSB as u8, /* index=10 */
    FLUID_MOD_CC   /* CC=1 */
        | FLUID_MOD_LINEAR  /* type=0 */
        | FLUID_MOD_BIPOLAR /* P=1 */
        | FLUID_MOD_POSITIVE, /* D=0 */
    0,
    0,       /* No second source */
    GEN_PAN, /* Target: pan */
    /* Amount: 500. The SF specs $8.4.6, p. 55 says: "Amount = 1000
    tenths of a percent". The center value (64) corresponds to 50%,
    so it follows that amount = 50% x 1000/% = 500. */
    500.0,
);

/* SF2.01 page 55 section 8.4.7: MIDI continuous controller 11 to initial attenuation*/
pub(crate) static DEFAULT_EXPR_MOD: FluidMod = def_mod(
    EXPRESSION_MSB as u8, /* index=11 */
    FLUID_MOD_CC          /* CC=1 */
        | FLUID_MOD_CONCAVE  /* type=1 */
        | FLUID_MOD_UNIPOLAR /* P=0 */
        | FLUID_MOD_NEGATIVE, /* D=1 */
    0,
    0,                             /* No second source */
    GEN_ATTENUATION,               /* Target: Initial attenuation */
    FLUID_PEAK_ATTENUATION as f64, /* Amount: 960 */
);

/* SF2.01 page 55 section 8.4.8: MIDI continuous controller 91 to Reverb send */
pub(crate) static DEFAULT_REVERB_MOD: FluidMod = def_mod(
    EFFECTS_DEPTH1 as u8, /* index=91 */
    FLUID_MOD_CC          /* CC=1 */
        | FLUID_MOD_LINEAR   /* type=0 */
        | FLUID_MOD_UNIPOLAR /* P=0 */
        | FLUID_MOD_POSITIVE, /* D=0 */
    0,
    0,              /* No second source */
    GEN_REVERBSEND, /* Target: Reverb send */
    200.0,          /* Amount: 200 ('tenths of a percent') */
);

/* SF2.01 page 55 section 8.4.9: MIDI continuous controller 93 to Chorus send */
pub(crate) static DEFAULT_CHORUS_MOD: FluidMod = def_mod(
    EFFECTS_DEPTH3 as u8, /* index=93 */
    FLUID_MOD_CC          /* CC=1 */
        | FLUID_MOD_LINEAR   /* type=0 */
        | FLUID_MOD_UNIPOLAR /* P=0 */
        | FLUID_MOD_POSITIVE, /* D=0 */
    0,
    0,              /* No second source */
    GEN_CHORUSSEND, /* Target: Chorus */
    200.0,          /* Amount: 200 ('tenths of a percent') */
);

/* SF2.01 page 57 section 8.4.10 MIDI Pitch Wheel to Initial Pitch ... */
/* Initial Pitch is not a "standard" generator, because it isn't mentioned in the
list of generators in the SF2 specifications. That's why destination Initial Pitch
is replaced here by fine tune generator.
 */
pub(crate) static DEFAULT_PITCH_BEND_MOD: FluidMod = def_mod(
    FLUID_MOD_PITCHWHEEL, /* Index=14 */
    FLUID_MOD_GC          /* CC =0 */
        | FLUID_MOD_LINEAR  /* type=0 */
        | FLUID_MOD_BIPOLAR /* P=1 */
        | FLUID_MOD_POSITIVE, /* D=0 */
    FLUID_MOD_PITCHWHEELSENS, /* Index = 16 */
    FLUID_MOD_GC              /* CC=0 */
        | FLUID_MOD_LINEAR   /* type=0 */
        | FLUID_MOD_UNIPOLAR /* P=0 */
        | FLUID_MOD_POSITIVE, /* D=0 */
    /* Also see the comment in gen.h about GEN_PITCH */
    GEN_FINETUNE, /* Destination: Fine Tune */
    12700.0,      /* Amount: 12700 cents */
);

/* Non-standard MIDI continuous controller 8 to channel stereo balance */
pub(crate) static CUSTOM_BALANCE_MOD: FluidMod = def_mod(
    BALANCE_MSB as u8, /* Index=8 */
    FLUID_MOD_CC       /* CC=1 */
        | FLUID_MOD_CONCAVE /* type=1 */
        | FLUID_MOD_BIPOLAR /* P=1 */
        | FLUID_MOD_POSITIVE, /* D=0 */
    0,
    0,
    GEN_CUSTOM_BALANCE, /* Destination: stereo balance */
    /* Amount: 96 dB of attenuation (on the opposite channel) */
    FLUID_PEAK_ATTENUATION as f64, /* Amount: 960 */
);

/* custom_breath2att_modulator is not a default modulator specified in SF2
 * it is intended to replace default_vel2att_mod on demand using
 * API fluid_set_breath_mode() or command shell setbreathmode.
 */
static CUSTOM_BREATH2ATT_MOD: FluidMod = def_mod(
    BREATH_MSB as u8, /* Source. breath MSB corresponds to 2. */
    FLUID_MOD_CC      /* MIDI continuous controller */
        | FLUID_MOD_CONCAVE  /* Curve shape. Corresponds to 'type=1' */
        | FLUID_MOD_UNIPOLAR /* Polarity. Corresponds to 'P=0' */
        | FLUID_MOD_NEGATIVE, /* Direction. Corresponds to 'D=1' */
    0,
    0,                             /* No 2nd source */
    GEN_ATTENUATION,               /* Target: Initial attenuation */
    FLUID_PEAK_ATTENUATION as f64, /* Modulation amount: 960 */
);

/***************************************************************
 *
 *               INITIALIZATION & UTILITIES
 */

/// Translation of `fluid_synth_settings()`.
pub(crate) fn fluid_synth_settings(settings: &mut crate::utils::settings::SettingsInner) {
    settings.register_int("synth.verbose", 0, 0, 1, FLUID_HINT_TOGGLED);

    settings.register_int("synth.reverb.active", 1, 0, 1, FLUID_HINT_TOGGLED);
    settings.register_num(
        "synth.reverb.room-size",
        FLUID_REVERB_DEFAULT_ROOMSIZE as f64,
        0.0,
        1.0,
        0,
    );
    settings.register_num(
        "synth.reverb.damp",
        FLUID_REVERB_DEFAULT_DAMP as f64,
        0.0,
        1.0,
        0,
    );
    settings.register_num(
        "synth.reverb.width",
        FLUID_REVERB_DEFAULT_WIDTH as f64,
        0.0,
        100.0,
        0,
    );
    settings.register_num(
        "synth.reverb.level",
        FLUID_REVERB_DEFAULT_LEVEL as f64,
        0.0,
        1.0,
        0,
    );

    settings.register_int("synth.chorus.active", 1, 0, 1, FLUID_HINT_TOGGLED);
    settings.register_int("synth.chorus.nr", FLUID_CHORUS_DEFAULT_N, 0, 99, 0);
    settings.register_num(
        "synth.chorus.level",
        FLUID_CHORUS_DEFAULT_LEVEL as f64,
        0.0,
        10.0,
        0,
    );
    settings.register_num(
        "synth.chorus.speed",
        FLUID_CHORUS_DEFAULT_SPEED as f64,
        0.1,
        5.0,
        0,
    );
    settings.register_num(
        "synth.chorus.depth",
        FLUID_CHORUS_DEFAULT_DEPTH as f64,
        0.0,
        256.0,
        0,
    );

    settings.register_int("synth.ladspa.active", 0, 0, 1, FLUID_HINT_TOGGLED);
    settings.register_int("synth.lock-memory", 1, 0, 1, FLUID_HINT_TOGGLED);
    settings.register_str("midi.portname", Some(""), 0);

    /* DEFAULT_SOUNDFONT */
    settings.register_str("synth.default-soundfont", Some(crate::DEFAULT_SOUNDFONT), 0);

    settings.register_int("synth.polyphony", 256, 1, 65535, 0);
    settings.register_int("synth.midi-channels", 16, 16, 256, 0);
    settings.register_num("synth.gain", 0.2, 0.0, 10.0, 0);
    settings.register_int("synth.audio-channels", 1, 1, 128, 0);
    settings.register_int("synth.audio-groups", 1, 1, 128, 0);
    settings.register_int("synth.effects-channels", 2, 2, 2, 0);
    settings.register_int("synth.effects-groups", 1, 1, 128, 0);
    settings.register_num("synth.sample-rate", 44100.0, 8000.0, 96000.0, 0);
    settings.register_int("synth.device-id", 16, 0, 127, 0);
    /* !ENABLE_MIXER_THREADS */
    settings.register_int("synth.cpu-cores", 1, 1, 1, 0);

    settings.register_int("synth.min-note-length", 10, 0, 65535, 0);

    settings.register_int(
        "synth.threadsafe-api",
        1, /* FLUID_THREAD_SAFE_CAPABLE */
        0,
        1,
        FLUID_HINT_TOGGLED,
    );

    settings.register_num("synth.overflow.percussion", 4000.0, -10000.0, 10000.0, 0);
    settings.register_num("synth.overflow.sustained", -1000.0, -10000.0, 10000.0, 0);
    settings.register_num("synth.overflow.released", -2000.0, -10000.0, 10000.0, 0);
    settings.register_num("synth.overflow.age", 1000.0, -10000.0, 10000.0, 0);
    settings.register_num("synth.overflow.volume", 500.0, -10000.0, 10000.0, 0);
    settings.register_num("synth.overflow.important", 5000.0, -50000.0, 50000.0, 0);
    settings.register_str("synth.overflow.important-channels", Some(""), 0);

    settings.register_str("synth.midi-bank-select", Some("gs"), 0);
    settings.add_option("synth.midi-bank-select", "gm");
    settings.add_option("synth.midi-bank-select", "gs");
    settings.add_option("synth.midi-bank-select", "xg");
    settings.add_option("synth.midi-bank-select", "mma");

    settings.register_int("synth.dynamic-sample-loading", 0, 0, 1, FLUID_HINT_TOGGLED);
    settings.register_int("synth.note-cut", 0, 0, 2, 0);

    settings.register_str("synth.portamento-time", Some("auto"), 0);
    settings.add_option("synth.portamento-time", "auto");
    settings.add_option("synth.portamento-time", "xg-gs");
    settings.add_option("synth.portamento-time", "linear");
}

/// Translation of `fluid_version()`: get FluidSynth runtime version.
pub fn fluid_version() -> (i32, i32, i32) {
    (
        crate::FLUIDSYNTH_VERSION_MAJOR,
        crate::FLUIDSYNTH_VERSION_MINOR,
        crate::FLUIDSYNTH_VERSION_MICRO,
    )
}

/// Translation of `fluid_version_str()`: get FluidSynth runtime version
/// as a string.
pub fn fluid_version_str() -> &'static str {
    crate::FLUIDSYNTH_VERSION
}

/* (fluid_synth_init(): the default modulators are the statics above, and
 * the dither table is only used by fluid_synth_write_s16(), which
 * SDL_mixer doesn't call) */

/// An event to the mixer (`fluid_synth_update_mixer()`).
#[inline]
fn fluid_synth_update_mixer(
    synth: &mut FluidSynthInner,
    method: RvoiceFunction,
    intparam: i32,
    realparam: FluidReal,
) {
    synth
        .eventhandler
        .push_int_real(method, RvoiceObject::Mixer, intparam, realparam);
}

/// `fluid_rvoice_eventhandler_push()`'s FLUID_OK or FLUID_FAILED.
#[inline]
fn push_result(ok: bool) -> i32 {
    if ok {
        FLUID_OK
    } else {
        FLUID_FAILED
    }
}

impl FluidSynthInner {
    /// Translation of `fluid_synth_get_ticks()`.
    #[inline]
    pub(crate) fn get_ticks(&self) -> u32 {
        self.ticks_since_start
    }

    /// Translation of `fluid_synth_add_ticks()`.
    #[inline]
    fn add_ticks(&mut self, val: i32) {
        self.ticks_since_start = self.ticks_since_start.wrapping_add(val as u32);
    }

    /// Translation of `fluid_sample_timer_process()`: called when
    /// synth->ticks is updated.
    fn fluid_sample_timer_process(&mut self) {
        let ticks = self.get_ticks();

        // (the timers are taken out of the synth while their callbacks
        // run on it)
        let mut timers = std::mem::take(&mut self.sample_timers);
        for st in timers.iter_mut() {
            if st.isfinished {
                continue;
            }

            let msec = (1000.0 * ((ticks as CULong).wrapping_sub(st.starttick) as f64)
                / self.sample_rate) as i64;
            let cont = match st.callback.as_mut() {
                Some(callback) => callback(self, msec as u32),
                None => 0,
            };

            if cont == 0 {
                st.isfinished = true;
            }
        }
        // (timers created by the callbacks go first, as upstream prepends)
        let added = std::mem::take(&mut self.sample_timers);
        self.sample_timers = added;
        self.sample_timers.extend(timers);
    }

    /// Translation of `new_fluid_sample_timer()`: returns the timer's ID.
    pub(crate) fn new_fluid_sample_timer(&mut self, callback: FluidSampleTimerCallback) -> u64 {
        self.next_sample_timer_id += 1;
        let id = self.next_sample_timer_id;
        let mut result = FluidSampleTimer {
            id,
            starttick: 0,
            callback: Some(callback),
            isfinished: false,
        };

        self.fluid_sample_timer_reset_timer(&mut result);
        self.sample_timers.insert(0, result);
        id
    }

    /// Translation of `delete_fluid_sample_timer()`.
    pub(crate) fn delete_fluid_sample_timer(&mut self, timer: u64) {
        if let Some(i) = self.sample_timers.iter().position(|t| t.id == timer) {
            self.sample_timers.remove(i);
        }
    }

    /// Translation of `fluid_sample_timer_reset()`.
    pub(crate) fn fluid_sample_timer_reset(&mut self, timer: u64) {
        let ticks = self.get_ticks();
        if let Some(t) = self.sample_timers.iter_mut().find(|t| t.id == timer) {
            t.starttick = ticks as CULong;
            t.isfinished = false;
        }
    }

    fn fluid_sample_timer_reset_timer(&self, timer: &mut FluidSampleTimer) {
        timer.starttick = self.get_ticks() as CULong;
        timer.isfinished = false;
    }

    /// Translation of `fluid_synth_get_min_note_length_LOCAL()`.
    #[inline]
    fn fluid_synth_get_min_note_length_local(&self) -> u32 {
        let mut i = 0;
        self.settings.getint("synth.min-note-length", &mut i);
        (i as f64 * self.sample_rate / 1000.0f32 as f64) as u32
    }

    /// The voice and what its functions reach through
    /// `voice->eventhandler` and `voice->channel`.
    pub(crate) fn split_voice(&mut self, v: usize) -> (&mut FluidVoice, VoiceEnv<'_>) {
        let voice = &mut self.voice[v];
        let channel = voice.channel.map(|c| &self.channel[c]);
        (
            voice,
            VoiceEnv {
                eh: &mut self.eventhandler,
                channel,
                active_voice_count: &mut self.active_voice_count,
                audio_groups: self.audio_groups,
                effects_groups: self.effects_groups,
                effects_channels: self.effects_channels,
                fromkey_portamento: self.fromkey_portamento,
                portamento_time_mode: self.portamento_time_mode,
                portamento_time_has_seen_lsb: self.portamento_time_has_seen_lsb,
                min_note_length_ticks: self.min_note_length_ticks,
            },
        )
    }
}

/// Translation of `new_fluid_synth()`: create new FluidSynth instance.
pub(crate) fn new_fluid_synth(settings: &Settings) -> Option<FluidSynthInner> {
    let mut i: i32 = 0;
    let prio_level = 0;
    let mut with_ladspa = 0;
    let mut sample_rate_min: f64 = 0.0;
    let mut sample_rate_max: f64 = 0.0;

    let mut use_mutex = 0;
    settings.getint("synth.threadsafe-api", &mut use_mutex);

    let mut with_reverb = 0;
    let mut with_chorus = 0;
    let mut verbose = 0;
    settings.getint("synth.reverb.active", &mut with_reverb);
    settings.getint("synth.chorus.active", &mut with_chorus);
    settings.getint("synth.verbose", &mut verbose);

    let mut polyphony = 0;
    let mut sample_rate = 0.0;
    let mut midi_channels = 0;
    let mut audio_channels = 0;
    let mut audio_groups = 0;
    let mut effects_channels = 0;
    let mut effects_groups = 0;
    let mut gain = 0.0f32;
    let mut device_id = 0;
    let mut cores = 0;
    settings.getint("synth.polyphony", &mut polyphony);
    settings.getnum("synth.sample-rate", &mut sample_rate);
    settings.getnum_range(
        "synth.sample-rate",
        &mut sample_rate_min,
        &mut sample_rate_max,
    );
    settings.getint("synth.midi-channels", &mut midi_channels);
    settings.getint("synth.audio-channels", &mut audio_channels);
    settings.getint("synth.audio-groups", &mut audio_groups);
    settings.getint("synth.effects-channels", &mut effects_channels);
    settings.getint("synth.effects-groups", &mut effects_groups);
    settings.getnum_float("synth.gain", &mut gain);
    settings.getint("synth.device-id", &mut device_id);
    settings.getint("synth.cpu-cores", &mut cores);

    let mut overflow = FluidOverflowPrio::default();
    settings.getnum_float("synth.overflow.percussion", &mut overflow.percussion);
    settings.getnum_float("synth.overflow.released", &mut overflow.released);
    settings.getnum_float("synth.overflow.sustained", &mut overflow.sustained);
    settings.getnum_float("synth.overflow.volume", &mut overflow.volume);
    settings.getnum_float("synth.overflow.age", &mut overflow.age);
    settings.getnum_float("synth.overflow.important", &mut overflow.important);

    settings.getint("synth.note-cut", &mut i);
    let msgs_note_cut_mode = i;

    /* register the callbacks */
    // (not here: see the top of this file)

    /* do some basic sanity checking on the settings */

    if midi_channels % 16 != 0 {
        let n = midi_channels / 16;
        midi_channels = (n + 1) * 16;
        settings.setint("synth.midi-channels", midi_channels);
        fluid_log(
            FLUID_WARN,
            "Requested number of MIDI channels is not a multiple of 16. I'll increase the number of channels to the next multiple.",
        );
    }

    if audio_channels < 1 {
        fluid_log(
            FLUID_WARN,
            "Requested number of audio channels is smaller than 1. Changing this setting to 1.",
        );
        audio_channels = 1;
    } else if audio_channels > 128 {
        flog!(
            FLUID_WARN,
            "Requested number of audio channels is too big ({}). Limiting this setting to 128.",
            audio_channels
        );
        audio_channels = 128;
    }

    if audio_groups < 1 {
        fluid_log(
            FLUID_WARN,
            "Requested number of audio groups is smaller than 1. Changing this setting to 1.",
        );
        audio_groups = 1;
    } else if audio_groups > 128 {
        flog!(
            FLUID_WARN,
            "Requested number of audio groups is too big ({}). Limiting this setting to 128.",
            audio_groups
        );
        audio_groups = 128;
    }

    if effects_channels < 2 {
        flog!(
            FLUID_WARN,
            "Invalid number of effects channels ({}).Setting effects channels to 2.",
            effects_channels
        );
        effects_channels = 2;
    }

    /*
    number of buffers rendered by the mixer is determined by synth->audio_groups.
    audio from MIDI channel is rendered, mapped and mixed in these buffers.

    Typically synth->audio_channels is only used by audio driver and should be set
    to the same value that synth->audio_groups. In some situation using LADSPA,
    it is best to diminish audio-channels so that the driver will be able to pass
    the audio to audio devices in the case these devices have a limited number of
    audio channels.

    audio-channels must not be greater then audio-groups, otherwise these
    audio output above audio-groups will not be rendered by the mixeur.
    */
    if audio_channels > audio_groups {
        audio_channels = audio_groups;
        settings.setint("synth.audio-channels", audio_channels);
        fluid_log(
            FLUID_WARN,
            "Requested audio-channels to high. Limiting this setting to audio-groups.",
        );
    }

    let _ = (use_mutex, cores, prio_level);

    /* Allocate event queue for rvoice mixer */
    /* In an overflow situation, a new voice takes about 50 spaces in the queue! */
    let eventhandler = FluidRvoiceEventhandler::new(
        polyphony * 64,
        polyphony,
        audio_groups,
        effects_channels,
        effects_groups,
        sample_rate_max as FluidReal,
        sample_rate,
    )?;

    // (the IIR filter's sin/cos table, filled at the end upstream, is
    // only read while rendering)
    let mut sincos_table = vec![FluidIirSincos::default(); SINCOS_TAB_SIZE];
    fluid_iir_filter_init_table(&mut sincos_table, sample_rate);

    let mut synth = FluidSynthInner {
        public_api_count: 0,
        settings: settings.clone(),
        device_id,
        polyphony,
        with_reverb,
        with_chorus,
        verbose,
        sample_rate,
        midi_channels,
        bank_select: 0,
        audio_channels,
        audio_groups,
        effects_channels,
        effects_groups,
        state: 0,
        ticks_since_start: 0,
        start: 0,
        overflow,
        loaders: Vec::new(),
        sfont: Vec::new(),
        sfont_id: 0,
        gain,
        channel: Vec::new(),
        nvoice: 0,
        voice: Vec::new(),
        active_voice_count: 0,
        noteid: 0,
        storeid: 0,
        fromkey_portamento: 0,
        eventhandler,
        reverb_param: [0.0; FLUID_REVERB_PARAM_LAST],
        chorus_param: [0.0; FLUID_CHORUS_PARAM_LAST],
        cur: 0,
        curmax: 0,
        dither_index: 0,
        cpu_load: 0.0,
        tuning: None,
        sample_timers: Vec::new(),
        next_sample_timer_id: 0,
        min_note_length_ticks: 0,
        cores,
        default_mod: Vec::new(),
        custom_filter_type: 0,
        custom_filter_flags: 0,
        msgs_note_cut_mode,
        portamento_time_mode: 0,
        portamento_time_has_seen_lsb: false,
        iir_sincos_table: Arc::from(sincos_table),
    };

    if let Some(important_channels) = settings.dupstr("synth.overflow.important-channels") {
        if synth.fluid_synth_set_important_channels(Some(&important_channels)) != FLUID_OK {
            fluid_log(FLUID_WARN, "Failed to set overflow important channels");
        }
    }

    /* as soon as the synth is created it starts playing. */
    synth.state = FLUID_SYNTH_PLAYING;

    synth.fromkey_portamento = INVALID_NOTE as i32; /* disable portamento */

    /* Initialize portamento time mode */
    {
        if let Some(portamento_mode_str) = settings.dupstr("synth.portamento-time") {
            let res = fluid_parse_portamento_time_str(&portamento_mode_str);
            if res == FLUID_FAILED {
                synth.portamento_time_mode = FLUID_PORTAMENTO_TIME_MODE_AUTO;
            } else {
                synth.portamento_time_mode = res;
            }
        } else {
            synth.portamento_time_mode = FLUID_PORTAMENTO_TIME_MODE_AUTO;
        }
        synth.portamento_time_has_seen_lsb = false; /* Start in xg-gs mode for auto */
    }

    synth.ticks_since_start = 0;
    synth.tuning = None;

    /* Setup the list of default modulators.
     * Needs to happen after eventhandler has been set up, as fluid_synth_enter_api is called in the process */
    synth.default_mod = Vec::new();
    synth.fluid_synth_add_default_mod(&DEFAULT_VEL2ATT_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&DEFAULT_VEL2FILTER_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&DEFAULT_AT2VIBLFO_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&DEFAULT_MOD2VIBLFO_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&DEFAULT_ATT_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&DEFAULT_PAN_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&DEFAULT_EXPR_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&DEFAULT_REVERB_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&DEFAULT_CHORUS_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&DEFAULT_PITCH_BEND_MOD, FLUID_SYNTH_ADD);
    synth.fluid_synth_add_default_mod(&CUSTOM_BALANCE_MOD, FLUID_SYNTH_ADD);

    /* Create and initialize the Fx unit.*/
    settings.getint("synth.ladspa.active", &mut with_ladspa);

    if with_ladspa != 0 {
        /* !LADSPA */
        fluid_log(
            FLUID_WARN,
            "FluidSynth has not been compiled with LADSPA support",
        );
    }

    /* allocate and add the dls sfont loader */
    /* ENABLE_NATIVE_DLS */
    let loader = SfLoader::new_fluid_dls_loader(settings);
    synth.fluid_synth_add_sfloader(loader);

    /* allocate and add the default sfont loader */
    let loader = SfLoader::new_defsfloader(settings);
    synth.fluid_synth_add_sfloader(loader);

    /* allocate all channel objects */
    if synth
        .channel
        .try_reserve_exact(midi_channels.max(0) as usize)
        .is_err()
    {
        fluid_log(FLUID_ERR, "Out of memory");
        return None;
    }
    for i in 0..synth.midi_channels {
        synth.channel.push(FluidChannel::new_uninit(i));
        // (new_fluid_channel())
        synth.fluid_channel_init(i as usize);
        synth.channel[i as usize].init_ctrl(false);
    }

    /* allocate all synthesis processes */
    synth.nvoice = synth.polyphony;
    if synth
        .voice
        .try_reserve_exact(synth.nvoice.max(0) as usize)
        .is_err()
    {
        return None;
    }
    for _ in 0..synth.nvoice {
        let voice = new_fluid_voice(
            &mut synth.eventhandler,
            synth.sample_rate,
            &synth.iir_sincos_table,
        )?;
        synth.voice.push(voice);
    }

    /* sets a default basic channel */
    /* Sets one basic channel: basic channel 0, mode 0 (Omni On - Poly) */
    /* (i.e all channels are polyphonic) */
    /* Must be called after channel objects allocation */
    let n = synth.midi_channels;
    synth.fluid_synth_set_basic_channel_local(0, FLUID_CHANNEL_MODE_OMNION_POLY, n);

    synth.min_note_length_ticks = synth.fluid_synth_get_min_note_length_local();

    let p = synth.polyphony;
    fluid_synth_update_mixer(&mut synth, RvoiceFunction::MixerSetPolyphony, p, 0.0);
    let r = synth.with_reverb;
    synth.fluid_synth_reverb_on(-1, r);
    let c = synth.with_chorus;
    synth.fluid_synth_chorus_on(-1, c);

    synth.cur = FLUID_BUFSIZE as i32;
    synth.curmax = 0;
    synth.dither_index = 0;

    {
        let mut values = [0.0f64; FLUID_REVERB_PARAM_LAST];

        settings.getnum("synth.reverb.room-size", &mut values[FLUID_REVERB_ROOMSIZE]);
        settings.getnum("synth.reverb.damp", &mut values[FLUID_REVERB_DAMP]);
        settings.getnum("synth.reverb.width", &mut values[FLUID_REVERB_WIDTH]);
        settings.getnum("synth.reverb.level", &mut values[FLUID_REVERB_LEVEL]);

        synth.fluid_synth_set_reverb_full(-1, FLUID_REVMODEL_SET_ALL, &values);
    }

    {
        let mut values = [0.0f64; FLUID_CHORUS_PARAM_LAST];

        settings.getint("synth.chorus.nr", &mut i);
        values[FLUID_CHORUS_NR] = i as f64;
        settings.getnum("synth.chorus.level", &mut values[FLUID_CHORUS_LEVEL]);
        settings.getnum("synth.chorus.speed", &mut values[FLUID_CHORUS_SPEED]);
        settings.getnum("synth.chorus.depth", &mut values[FLUID_CHORUS_DEPTH]);
        values[FLUID_CHORUS_TYPE] = FLUID_CHORUS_DEFAULT_TYPE as f64;

        synth.fluid_synth_set_chorus_full(-1, FLUID_CHORUS_SET_ALL, &values);
    }

    synth.bank_select = FLUID_BANK_STYLE_GS;

    if settings.str_equal("synth.midi-bank-select", "gm") {
        synth.bank_select = FLUID_BANK_STYLE_GM;
    } else if settings.str_equal("synth.midi-bank-select", "gs") {
        synth.bank_select = FLUID_BANK_STYLE_GS;
    } else if settings.str_equal("synth.midi-bank-select", "xg") {
        synth.bank_select = FLUID_BANK_STYLE_XG;
    } else if settings.str_equal("synth.midi-bank-select", "mma") {
        synth.bank_select = FLUID_BANK_STYLE_MMA;
    }

    synth.fluid_synth_process_event_queue();

    /* FIXME */
    synth.start = fluid_curtime();

    Some(synth)
}

impl Drop for FluidSynthInner {
    /// The part of `delete_fluid_synth()` that is not the `Drop`s of its
    /// members: turn off all voices, needed to unload SoundFont data.
    fn drop(&mut self) {
        for v in 0..self.voice.len() {
            let (voice, env) = self.split_voice(v);
            /* WARNING: A this point we must ensure that the reference counter
            of any soundfont sample owned by any rvoice belonging to the voice
            are correctly decremented. This is the contrary part to
            to fluid_voice_init() where the sample's reference counter is
            incremented.
            */
            voice.can_access_rvoice = true; /* fluid_voice_unlock_rvoice() */
            fluid_voice_overflow_rvoice_finished(voice);

            if fluid_voice_is_playing(voice) {
                fluid_voice_off(voice, env.eh);
                /* If we only use fluid_voice_off(voice) it will trigger a delayed
                 * fluid_voice_stop(voice) via fluid_synth_check_finished_voices().
                 * But here, we are deleting the fluid_synth_t instance so
                 * fluid_voice_stop() will be never triggered resulting in
                 * SoundFont data never unloaded (i.e a serious memory leak).
                 * So, fluid_voice_stop() must be explicitly called to insure
                 * unloading SoundFont data
                 */
                fluid_voice_stop(voice, env.active_voice_count);
            }
        }

        /* also unset all presets for clean SoundFont unload */
        for channel in self.channel.iter_mut() {
            channel.set_preset(None);
        }
    }
}

impl FluidSynthInner {
    /// Translation of `fluid_synth_api_enter()`.
    pub(crate) fn fluid_synth_api_enter(&mut self) {
        // (the mutex is the Synth handle's)
        if self.public_api_count == 0 {
            self.fluid_synth_check_finished_voices();
        }

        self.public_api_count += 1;
    }

    /// Translation of `fluid_synth_api_exit()`.
    pub(crate) fn fluid_synth_api_exit(&mut self) {
        self.public_api_count -= 1;

        if self.public_api_count == 0 {
            self.eventhandler.flush();
        }
    }

    /// `FLUID_API_ENTRY_CHAN()`: false when the API function returns its
    /// fail value (chan < 0, or past the MIDI channels after entering).
    fn api_entry_chan(&mut self, chan: i32) -> bool {
        if chan < 0 {
            return false;
        }
        self.fluid_synth_api_enter();
        if chan >= self.midi_channels {
            self.fluid_synth_api_exit();
            return false;
        }
        true
    }

    /// `FLUID_API_RETURN_IF_CHAN_DISABLED()`: false (after leaving the
    /// API) when the channel is disabled.
    fn api_chan_enabled(&mut self, chan: i32) -> bool {
        if self.channel[chan as usize].mode & FLUID_CHANNEL_ENABLED != 0 {
            true
        } else {
            flog!(FLUID_INFO, "Channel {} is disabled, event dropped!", chan);
            self.fluid_synth_api_exit();
            false
        }
    }

    /// `FLUID_API_RETURN(return_value)`
    #[inline]
    fn api_return<T>(&mut self, return_value: T) -> T {
        self.fluid_synth_api_exit();
        return_value
    }

    /// Translation of `fluid_synth_noteon()`: send a note-on event to a
    /// FluidSynth object.
    ///
    /// This function will take care of proper legato playing. If a note on channel @p chan is
    /// already playing at the given key @p key, it will be released (even if it is sustained).
    /// In other words, overlapping notes are not allowed.
    pub(crate) fn fluid_synth_noteon(&mut self, chan: i32, key: i32, vel: i32) -> i32 {
        if !(0..=127).contains(&key) || !(0..=127).contains(&vel) {
            return FLUID_FAILED;
        }
        if !self.api_entry_chan(chan) {
            return FLUID_FAILED;
        }

        /* Allowed only on MIDI channel enabled */
        if !self.api_chan_enabled(chan) {
            return FLUID_FAILED;
        }

        let result = self.fluid_synth_noteon_local(chan, key, vel);
        self.api_return(result)
    }

    /// Translation of `fluid_synth_noteon_LOCAL()`: local synthesis thread
    /// variant of fluid_synth_noteon.
    fn fluid_synth_noteon_local(&mut self, chan: i32, key: i32, vel: i32) -> i32 {
        /* notes with velocity zero go to noteoff  */
        if vel == 0 {
            return self.fluid_synth_noteoff_local(chan, key);
        }

        let channel = &self.channel[chan as usize];

        /* makes sure this channel has a preset */
        if channel.preset.is_none() {
            if self.verbose != 0 {
                flog!(
                    FLUID_INFO,
                    "noteon\t{}\t{}\t{}\t{:05}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}",
                    chan,
                    key,
                    vel,
                    0,
                    self.get_ticks() as f32 / 44100.0f32,
                    (fluid_curtime().wrapping_sub(self.start)) as f32 / 1000.0f32,
                    0.0f32,
                    0,
                    "channel has no preset"
                );
            }

            return FLUID_FAILED;
        }

        if channel.is_playing_mono() {
            /* channel is mono or legato CC is On) */
            /* play the noteOn in monophonic */
            self.fluid_synth_noteon_mono_local(chan, key, vel)
        } else {
            /* channel is poly and legato CC is Off) */

            /* plays the noteOn in polyphonic */
            /* Sets the note at first position in monophonic list */
            /* In the case where the musician intends to inter the channel in monophonic
            (by depressing the CC legato on), the next noteOn mono could be played legato
             with the previous note poly (if the musician choose this).
                */
            self.channel[chan as usize].set_onenote_monolist(key as u8, vel as u8);

            /* If there is another voice process on the same channel and key,
            advance it to the release phase. */
            self.fluid_synth_release_voice_on_same_note_local(chan, key);

            /* a noteon poly is passed to fluid_synth_noteon_monopoly_legato().
            This allows an opportunity to get this note played legato with a previous
            note if a CC PTC have been received before this noteon. This behavior is
            a MIDI specification (see FluidPolymono-0004.pdf chapter 4.3-a ,3.4.11
            for details).
            */
            self.fluid_synth_noteon_monopoly_legato(chan, INVALID_NOTE as i32, key, vel)
        }
    }

    /// Translation of `fluid_synth_noteoff()`: sends a note-off event to a
    /// FluidSynth object.
    ///
    /// @return #FLUID_OK on success, #FLUID_FAILED otherwise (may just mean that no
    ///   voices matched the note off event)
    pub(crate) fn fluid_synth_noteoff(&mut self, chan: i32, key: i32) -> i32 {
        if !(0..=127).contains(&key) {
            return FLUID_FAILED;
        }
        if !self.api_entry_chan(chan) {
            return FLUID_FAILED;
        }

        /* Allowed only on MIDI channel enabled */
        if !self.api_chan_enabled(chan) {
            return FLUID_FAILED;
        }

        let result = self.fluid_synth_noteoff_local(chan, key);
        self.api_return(result)
    }

    /// Translation of `fluid_synth_noteoff_LOCAL()`: local synthesis
    /// thread variant of fluid_synth_noteoff.
    fn fluid_synth_noteoff_local(&mut self, chan: i32, key: i32) -> i32 {
        let status;
        let channel = &mut self.channel[chan as usize];

        if channel.is_playing_mono() {
            /* channel is mono or legato CC is On) */
            /* play the noteOff in monophonic */
            status = self.fluid_synth_noteoff_mono_local(chan, key);
        } else {
            /* channel is poly and legato CC is Off) */
            /* removes the note from the monophonic list */
            if channel.n_notes != 0 && key == channel.last_note() as i32 {
                channel.clear_monolist();
            }

            status = self.fluid_synth_noteoff_monopoly(chan, key, false);
        }

        /* Changes the state (Valid/Invalid) of the most recent note played in a
        staccato manner */
        self.channel[chan as usize].invalid_prev_note_staccato();
        status
    }

    /// Translation of `fluid_synth_damp_voices_by_sustain_LOCAL()`: damps
    /// voices on a channel (turn notes off), if they're sustained by
    /// sustain pedal.
    fn fluid_synth_damp_voices_by_sustain_local(&mut self, chan: i32) -> i32 {
        for i in 0..self.polyphony as usize {
            let voice = &self.voice[i];

            if (fluid_voice_get_channel(voice) == chan) && fluid_voice_is_sustained(voice) {
                if voice.key == self.channel[chan as usize].key_mono_sustained {
                    /* key_mono_sustained is a possible mono note sustainted
                    (by sustain or sostenuto pedal). It must be marked released
                    (INVALID_NOTE) here because it is released only by sustain pedal */
                    self.channel[chan as usize].key_mono_sustained = INVALID_NOTE;
                }

                let (voice, mut env) = self.split_voice(i);
                fluid_voice_release(voice, &mut env);
            }
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_damp_voices_by_sostenuto_LOCAL()`:
    /// damps voices on a channel (turn notes off), if they're sustained by
    /// sostenuto pedal.
    fn fluid_synth_damp_voices_by_sostenuto_local(&mut self, chan: i32) -> i32 {
        for i in 0..self.polyphony as usize {
            let voice = &self.voice[i];

            if (fluid_voice_get_channel(voice) == chan) && fluid_voice_is_sostenuto(voice) {
                if voice.key == self.channel[chan as usize].key_mono_sustained {
                    /* key_mono_sustained is a possible mono note sustainted
                    (by sustain or sostenuto pedal). It must be marked released
                    (INVALID_NOTE) here because it is released only by sostenuto pedal */
                    self.channel[chan as usize].key_mono_sustained = INVALID_NOTE;
                }

                let (voice, mut env) = self.split_voice(i);
                fluid_voice_release(voice, &mut env);
            }
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_add_default_mod()`: adds the specified
    /// modulator \c mod as default modulator to the synth. \c mod will
    /// take effect for any subsequently created voice.
    /// @param mode Determines how to handle an existing identical modulator (#fluid_synth_add_mod)
    pub(crate) fn fluid_synth_add_default_mod(&mut self, mod_: &FluidMod, mode: i32) -> i32 {
        if !((mode == FLUID_SYNTH_ADD) || (mode == FLUID_SYNTH_OVERWRITE)) {
            return FLUID_FAILED;
        }

        /* Checks if modulators sources are valid */
        if !fluid_mod_check_sources(mod_, Some("api fluid_synth_add_default_mod mod")) {
            return FLUID_FAILED;
        }

        self.fluid_synth_api_enter();

        for default_mod in self.default_mod.iter_mut() {
            if fluid_mod_test_identity(default_mod, mod_) {
                if mode == FLUID_SYNTH_ADD {
                    default_mod.amount += mod_.amount;
                } else {
                    // mode == FLUID_SYNTH_OVERWRITE
                    default_mod.amount = mod_.amount;
                }

                return self.api_return(FLUID_OK);
            }
        }

        /* Add a new modulator (no existing modulator to add / overwrite). */
        let mut new_mod = new_fluid_mod();
        new_mod.clone_from_mod(mod_);
        self.default_mod.push(new_mod);

        self.api_return(FLUID_OK)
    }

    /// Translation of `fluid_synth_cc()`: send a MIDI controller event on
    /// a MIDI channel.
    ///
    /// Most CCs are 7-bits wide in FluidSynth. There are a few exceptions which may be 14-bits wide as are documented here:
    /// https://github.com/FluidSynth/fluidsynth/wiki/FluidFeatures#midi-control-change-implementation-chart
    ///
    /// @note This function supports MIDI Global Controllers which will be sent to
    /// all channels of the basic channel if this basic channel is in mode OmniOff/Mono.
    /// This is accomplished by sending the CC one MIDI channel below the basic
    /// channel of the receiver.
    /// Examples: let a synthesizer with 16 MIDI channels:
    /// - Let a basic channel 7 in mode 3 (Omni Off, Mono). If MIDI channel 6 is disabled it
    ///   could be used as CC global for all channels belonging to basic channel 7.
    /// - Let a basic channel 0 in mode 3. If MIDI channel 15  is disabled it could be used
    ///   as CC global for all channels belonging to basic channel 0.
    ///
    /// @warning Contrary to the MIDI Standard, this function does not clear LSB controllers,
    /// when MSB controllers are received.
    pub(crate) fn fluid_synth_cc(&mut self, chan: i32, num: i32, val: i32) -> i32 {
        let mut result = FLUID_FAILED;
        if !(0..=127).contains(&num) || !(0..=127).contains(&val) {
            return FLUID_FAILED;
        }
        if !self.api_entry_chan(chan) {
            return FLUID_FAILED;
        }

        let channel = &mut self.channel[chan as usize];

        if channel.mode & FLUID_CHANNEL_ENABLED != 0 {
            /* chan is enabled */
            if self.verbose != 0 {
                flog!(FLUID_INFO, "cc\t\t{}\t{}\t{}", chan, num, val);
            }

            self.channel[chan as usize].set_cc(num, val);
            result = self.fluid_synth_cc_local(chan, num);
        } else {
            /* chan is disabled so it is a candidate for global channel */
            /* looks for next basic channel */
            let n_chan = self.midi_channels; /* MIDI Channels number */
            let basicchan = if chan < n_chan - 1 {
                chan + 1 /* next channel */
            } else {
                0 /* wrap to 0 */
            };

            let channel = &self.channel[basicchan as usize];

            /* Channel must be a basicchan in mode OMNIOFF_MONO */
            if (channel.mode & FLUID_CHANNEL_BASIC) != 0
                && ((channel.mode & FLUID_CHANNEL_MODE_MASK) == FLUID_CHANNEL_MODE_OMNIOFF_MONO)
            {
                /* sends cc to all channels in this basic channel */
                let nbr = channel.mode_val;

                for i in basicchan..basicchan + nbr {
                    if self.verbose != 0 {
                        flog!(FLUID_INFO, "cc\t\t{}\t{}\t{}", i, num, val);
                    }

                    self.channel[i as usize].set_cc(num, val);
                    result = self.fluid_synth_cc_local(i, num);
                }
            }
            /* The channel chan is not a valid 'global channel' */
            else {
                flog!(
                    FLUID_INFO,
                    "Ignoring cc{}={} - channel {} is disabled and not a valid global channel either!",
                    num,
                    val,
                    chan
                );
                result = FLUID_FAILED;
            }
        }

        self.api_return(result)
    }

    /// Translation of `fluid_synth_cc_LOCAL()`: local synthesis thread
    /// variant of MIDI CC set function.
    ///
    /// Most of CC are allowed to modulate but not all. A comment describes if CC num
    /// isn't allowed to modulate.
    /// Following explanations should help to understand both MIDI specifications and
    /// Soundfont specifications in regard to MIDI specs.
    ///
    /// MIDI specs:
    /// CC LSB (32 to 63) are LSB contributions to CC MSB (0 to 31).
    /// It's up to the synthesizer to decide to take LSB values into account or not.
    /// Actually Fluidsynth doesn't use CC LSB value inside fluid_voice_update_param()
    /// (once fluid_voice_modulate() has been triggered). This is because actually
    /// fluidsynth needs only 7 bits resolution (and not 14 bits) from these CCs.
    /// So fluidsynth is using only 7 bit MSB (except for portamento time).
    /// In regard to MIDI specs Fluidsynth behaves correctly.
    ///
    /// Soundfont specs 2.01 - 8.2.1:
    /// To deal correctly with MIDI CC (regardless if any synth will use CC MSB alone (7 bit)
    /// or both CCs MSB,LSB (14 bits) during synthesis), SF specs recommend not making use of
    /// CC LSB (i.e only CC MSB) in modulator sources to trigger modulation (i.e modulators
    /// with CC LSB connected to sources inputs should be ignored).
    /// These specifics are particularly suited for synths that use 14 bits CCs. In this case,
    /// the MIDI transmitter sends CC LSB first followed by CC MSB. The MIDI synth receives
    /// both CC LSB and CC MSB but only CC MSB will trigger the modulation.
    /// This will produce correct synthesis parameters update from a correct 14 bits CC.
    /// If in SF specs, modulator sources with CC LSB had been accepted, both CC LSB and
    /// CC MSB will triggers 2 modulations. This leads to incorrect synthesis parameters
    /// update followed by correct synthesis parameters update.
    ///
    /// However, as long as fluidsynth will use only CC 7 bits resolution, it is safe to ignore
    /// these SF recommendations on CC receive.
    pub(crate) fn fluid_synth_cc_local(&mut self, channum: i32, num: i32) -> i32 {
        let c = channum as usize;
        let mut value = self.channel[c].get_cc(num);

        match num {
            LOCAL_CONTROL => { /* not allowed to modulate (spec SF 2.01 - 8.2.1) */ }

            /* CC omnioff, omnion, mono, poly */
            /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
            POLY_OFF | POLY_ON | OMNI_OFF | OMNI_ON => {
                /* allowed only if channum is a basic channel */
                if self.channel[c].mode & FLUID_CHANNEL_BASIC != 0 {
                    /* Construction of new_mode from current channel mode and this CC mode */
                    let mut new_mode = self.channel[c].mode & FLUID_CHANNEL_MODE_MASK;

                    match num {
                        POLY_OFF => new_mode |= FLUID_CHANNEL_POLY_OFF,
                        POLY_ON => new_mode &= !FLUID_CHANNEL_POLY_OFF,
                        OMNI_OFF => new_mode |= FLUID_CHANNEL_OMNI_OFF,
                        OMNI_ON => new_mode &= !FLUID_CHANNEL_OMNI_OFF,
                        _ => {
                            /* should never happen */
                            return FLUID_FAILED;
                        }
                    }

                    /* MIDI specs: if value is 0 it means all channels from channum to next
                    basic channel minus 1 (if any) or to MIDI channel count minus 1.
                    However, if value is > 0 (e.g. 4), the group of channels will be be
                    limited to 4.
                    value is ignored for #FLUID_CHANNEL_MODE_OMNIOFF_POLY as this mode
                    implies a group of only one channel.
                    */
                    /* Checks value range and changes this existing basic channel group */
                    value = self.fluid_synth_check_next_basic_channel(channum, new_mode, value);

                    if value != FLUID_FAILED {
                        /* reset the current basic channel before changing it */
                        let mode_val = self.channel[c].mode_val;
                        self.fluid_synth_reset_basic_channel_local(channum, mode_val);
                        self.fluid_synth_set_basic_channel_local(channum, new_mode, value);
                        flog!(
                            FLUID_INFO,
                            "Successfully configured channel group, from channel {} up to including chan {} to mode 0x{:X}",
                            channum,
                            channum + value,
                            new_mode
                        );
                        return FLUID_OK; /* FLUID_OK */
                    }
                } else {
                    const POLY_MONO_STR: [&str; 4] = ["OMNI_OFF", "OMNI_ON", "POLY_OFF", "POLY_ON"];
                    flog!(
                        FLUID_WARN,
                        "Failed to set channel {} to {}: Operation illegal, because channel is currently no basic channel.",
                        channum,
                        POLY_MONO_STR[(num - OMNI_OFF) as usize]
                    );
                }

                return FLUID_FAILED;
            }

            LEGATO_SWITCH => {
                /* not allowed to modulate */
                /* handles Poly/mono commutation on Legato pedal On/Off.*/
                self.fluid_channel_cc_legato(c, value);
            }

            PORTAMENTO_SWITCH => {
                /* not allowed to modulate */
                /* Special handling of the monophonic list  */
                /* Invalids the most recent note played in a staccato manner */
                self.channel[c].invalid_prev_note_staccato();
            }

            SUSTAIN_SWITCH => {
                /* not allowed to modulate */

                /* Release voices if Sustain switch is released */
                if value < 64 {
                    /* Sustain is released */
                    self.fluid_synth_damp_voices_by_sustain_local(channum);
                }
            }

            SOSTENUTO_SWITCH => {
                /* not allowed to modulate */

                /* Release voices if Sostetuno switch is released */
                if value < 64 {
                    /* Sostenuto is released */
                    self.fluid_synth_damp_voices_by_sostenuto_local(channum);
                } else {
                    /* Sostenuto is depressed */
                    /* Update sostenuto order id when pedaling on Sostenuto */
                    self.channel[c].sostenuto_orderid = self.noteid; /* future voice id value */
                }
            }

            BANK_SELECT_MSB => {
                /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
                let bank_select = self.bank_select;
                self.channel[c].set_bank_msb(bank_select, value & 0x7F);
            }

            BANK_SELECT_LSB => {
                /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
                let bank_select = self.bank_select;
                self.channel[c].set_bank_lsb(bank_select, value & 0x7F);
            }

            ALL_NOTES_OFF => {
                /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
                self.fluid_synth_all_notes_off_local(channum);
            }

            ALL_SOUND_OFF => {
                /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
                self.fluid_synth_all_sounds_off_local(channum);
            }

            ALL_CTRL_OFF => {
                /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
                self.channel[c].init_ctrl(true);
                // the hold pedals have been reset, we maybe need to release voices
                self.fluid_synth_damp_voices_by_sustain_local(channum);
                self.fluid_synth_damp_voices_by_sostenuto_local(channum);
                self.fluid_synth_modulate_voices_all_local(channum);
            }

            DATA_ENTRY_LSB | DATA_ENTRY_MSB => {
                /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
                /* handle both because msb might come first */
                let lsb_value = self.channel[c].get_cc(DATA_ENTRY_LSB);
                let msb_value = self.channel[c].get_cc(DATA_ENTRY_MSB);
                let data = (msb_value << 7) + lsb_value;

                if self.channel[c].nrpn_active != 0 {
                    /* NRPN is active? */
                    /* SontFont 2.01 NRPN Message (Sect. 9.6, p. 74)  */
                    if (self.channel[c].get_cc(NRPN_MSB) == 120)
                        && (self.channel[c].get_cc(NRPN_LSB) < 100)
                    {
                        if num == DATA_ENTRY_LSB {
                            // SF2 NRPNs must be processed, after having received DATA_LSB and DATA_MSB in that order.
                            return FLUID_OK;
                        }
                        let nrpn_select = self.channel[c].nrpn_select;

                        // (nrpn_select only goes negative after 2^31 NRPN LSB
                        // messages, which index out of bounds upstream)
                        if (0..GEN_LAST as i32).contains(&nrpn_select) {
                            let val = fluid_gen_scale_nrpn(nrpn_select as usize, data) as f32;
                            if self.verbose != 0 {
                                flog!(
                                    FLUID_INFO,
                                    "NRPN\t{}\t{}\t{}\t{:.6}",
                                    channum,
                                    nrpn_select,
                                    data,
                                    val
                                );
                            }
                            self.fluid_synth_set_gen_local(channum, nrpn_select as usize, val);
                        }

                        self.channel[c].nrpn_select = 0; /* Reset to 0 */
                    } else if self.channel[c].get_cc(NRPN_MSB) == 127 {
                        // indicates AWE32 NRPNs
                        // ALTITUDE.MID also manipulates AWE32 NRPNs by only using DATA LSB events - seems to be legal
                        if self.channel[c].get_cc(NRPN_MSB) == 127 {
                            // indicates AWE32 NRPNs
                            let gen = self.channel[c].get_cc(NRPN_LSB);
                            if self.verbose != 0 {
                                flog!(
                                    FLUID_INFO,
                                    "AWE32 NRPN RAW: Chan {}, Gen {}, data {} | 0x{:X}, MSB: {}, LSB: {}",
                                    channum,
                                    gen,
                                    data,
                                    data,
                                    msb_value,
                                    lsb_value
                                );
                            }
                            if gen <= 26 {
                                // Effect 26 (reverb) is the last effect to select
                                self.fluid_synth_process_awe32_nrpn_local(
                                    channum, gen, data, lsb_value,
                                );
                            } else {
                                flog!(
                                    FLUID_INFO,
                                    "Ignoring unknown AWE32 NRPN targeting effect {}",
                                    gen
                                );
                            }
                        }
                    } else if self.channel[c].get_cc(NRPN_MSB) == 1
                        && self.bank_select == FLUID_BANK_STYLE_GS
                    {
                        let mut nrpn2cc = -1;
                        let nrpn_lsb = self.channel[c].get_cc(NRPN_LSB);
                        // cf. SC8850 owner's manual pages 227 + 228
                        match nrpn_lsb {
                            8 => {
                                // vibrato rate
                                nrpn2cc = SOUND_CTRL7;
                            }
                            9 => {
                                // vibrato depth
                                nrpn2cc = SOUND_CTRL8;
                            }
                            10 => {
                                // vibrato rate
                                nrpn2cc = SOUND_CTRL9;
                            }
                            _ => {}
                        }
                        if nrpn2cc != -1 {
                            if self.verbose != 0 {
                                flog!(
                                    FLUID_INFO,
                                    "Translating Roland GS NRPN {} to CC {}",
                                    nrpn_lsb,
                                    nrpn2cc
                                );
                            }
                            self.fluid_synth_cc(channum, nrpn2cc, msb_value);
                        } else if self.verbose != 0 {
                            flog!(FLUID_INFO, "Ignoring unknown Roland GS NRPN {}", nrpn_lsb);
                        }
                    }
                } else if self.channel[c].get_cc(RPN_MSB) == 0 {
                    /* RPN is active: MSB = 0? */
                    match self.channel[c].get_cc(RPN_LSB) {
                        RPN_PITCH_BEND_RANGE => {
                            /* Set bend range in semitones plus cents */
                            self.channel[c].set_pitch_wheel_sensitivity(
                                msb_value as f32 + lsb_value as f32 / 100.0f32,
                            ); /* 0-127 maps to 0-100 cents */
                            self.fluid_synth_update_pitch_wheel_sens_local(channum);
                            /* Update bend range */
                        }

                        RPN_CHANNEL_FINE_TUNE => {
                            /* Fine tune is 14 bit over +/-1 semitone (+/- 100 cents, 8192 = center) */
                            self.fluid_synth_set_gen_local(
                                channum,
                                GEN_FINETUNE,
                                (data - 8192) as f32 * (100.0f32 / 8192.0f32),
                            );
                        }

                        RPN_CHANNEL_COARSE_TUNE => {
                            /* Coarse tune is 7 bit and in semitones (64 is center) */
                            self.fluid_synth_set_gen_local(
                                channum,
                                GEN_COARSETUNE,
                                (msb_value - 64) as f32,
                            );
                        }

                        RPN_TUNING_PROGRAM_CHANGE => {
                            self.channel[c].set_tuning_prog(msb_value);
                            let bank = self.channel[c].get_tuning_bank();
                            self.fluid_synth_activate_tuning(channum, bank, msb_value, true);
                        }

                        RPN_TUNING_BANK_SELECT => {
                            self.channel[c].set_tuning_bank(msb_value);
                        }

                        RPN_MODULATION_DEPTH_RANGE => {}

                        _ => {}
                    }
                }
            }

            NRPN_MSB => {
                /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
                let chan = &mut self.channel[c];
                chan.set_cc(NRPN_LSB, 0);
                chan.set_cc(DATA_ENTRY_MSB, 0);
                chan.set_cc(DATA_ENTRY_LSB, 0);
                chan.nrpn_select = 0;
                chan.nrpn_active = 1;
            }

            NRPN_LSB => {
                /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
                let chan = &mut self.channel[c];
                chan.set_cc(DATA_ENTRY_MSB, 0);
                chan.set_cc(DATA_ENTRY_LSB, 0);
                /* SontFont 2.01 NRPN Message (Sect. 9.6, p. 74)  */
                if chan.get_cc(NRPN_MSB) == 120 {
                    if value == 100 {
                        chan.nrpn_select = chan.nrpn_select.wrapping_add(100);
                    } else if value == 101 {
                        chan.nrpn_select = chan.nrpn_select.wrapping_add(1000);
                    } else if value == 102 {
                        chan.nrpn_select = chan.nrpn_select.wrapping_add(10000);
                    } else if value < 100 {
                        chan.nrpn_select = chan.nrpn_select.wrapping_add(value);
                    }
                }

                chan.nrpn_active = 1;
            }

            RPN_MSB | RPN_LSB => {
                /* not allowed to modulate (spec SF 2.01 - 8.2.1) */
                let chan = &mut self.channel[c];
                chan.nrpn_active = 0;
                chan.set_cc(DATA_ENTRY_MSB, 0);
                chan.set_cc(DATA_ENTRY_LSB, 0);
            }

            _ => {
                if num == BREATH_MSB {
                    /* handles CC Breath On/Off noteOn/noteOff mode */
                    self.fluid_channel_cc_breath_note_on_off(c, value);
                }

                /* fall-through */
                if num == BREATH_MSB || num == PORTAMENTO_TIME_LSB {
                    /* Track LSB reception for auto portamento mode */
                    if num == PORTAMENTO_TIME_LSB
                        && self.portamento_time_mode == FLUID_PORTAMENTO_TIME_MODE_AUTO
                    {
                        self.portamento_time_has_seen_lsb = true;
                    }
                }
                /* fall-through */
                /* CC lsb shouldn't allowed to modulate (spec SF 2.01 - 8.2.1) */
                /* However, as long fluidsynth will use only CC 7 bits resolution, it
                is safe to ignore these SF recommendations on CC receive. See
                explanations above */
                /* if (! (32 <= num && num <= 63)) */
                {
                    return self.fluid_synth_modulate_voices_local(channum, true, num);
                }
            }
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_sysex()`: process a MIDI SYSEX (system
    /// exclusive) message.
    ///
    /// @param data Buffer containing SYSEX data (not including 0xF0 and 0xF7)
    /// @param handled Optional location to store boolean value if message was
    ///   recognized and handled or not (set to TRUE if it was handled)
    /// @param dryrun TRUE to just do a dry run but not actually execute the SYSEX
    ///   command (useful for checking if a SYSEX message would be handled)
    ///
    /// The response buffer of upstream's function (for bulk tuning dump
    /// requests) is left out: requests get no response, as upstream's
    /// with a NULL response buffer, which is how its MIDI player calls it.
    /// The data bytes are signed (`char` on x86 and Apple platforms).
    ///
    /// @note When Fluidsynth receives an XG System Mode ON message, it compares the @p synth 's deviceID
    /// directly with the deviceID of the SysEx message. This is contrary to the XG spec (page 42), which
    /// requires to only compare the lower nibble. However, following the XG spec seems to break drum channels
    /// for a lot of MIDI files out there and therefore we've decided for this customization. If you rely on
    /// XG System Mode ON messages, make sure to set the setting \ref settings_synth_device-id to match the
    /// deviceID provided in the SysEx message (in most cases, this will be <code>deviceID=16</code>).
    ///
    /// SYSEX format (0xF0 and 0xF7 bytes shall not be passed to this function):
    /// Non-realtime:    0xF0   0x7E      <DeviceId> [BODY] 0xF7
    /// Realtime:        0xF0   0x7F      <DeviceId> [BODY] 0xF7
    /// Tuning messages: 0xF0   0x7E/0x7F <DeviceId> 0x08 <sub ID2> [BODY] <ChkSum> 0xF7
    /// GS DT1 messages: 0xF0   0x41      <DeviceId> 0x42 0x12 [ADDRESS (3 bytes)] [DATA] <ChkSum> 0xF7
    pub(crate) fn fluid_synth_sysex(
        &mut self,
        data: &[i8],
        mut handled: Option<&mut bool>,
        dryrun: bool,
    ) -> i32 {
        if let Some(h) = handled.as_deref_mut() {
            *h = false;
        }

        let len = data.len() as i32;
        if len <= 0 {
            return FLUID_FAILED;
        }

        if len < 4 {
            return FLUID_OK;
        }

        let d = |i: usize| data[i] as i32;
        let device_id = self.device_id;
        let dev_ok = d(1) == device_id
            || d(1) == MIDI_SYSEX_DEVICE_ID_ALL
            || device_id == MIDI_SYSEX_DEVICE_ID_ALL;

        /* MIDI tuning SYSEX message? */
        if (d(0) == MIDI_SYSEX_UNIV_NON_REALTIME || d(0) == MIDI_SYSEX_UNIV_REALTIME)
            && dev_ok
            && d(2) == MIDI_SYSEX_MIDI_TUNING_ID
        {
            self.fluid_synth_api_enter();
            let result = self.fluid_synth_sysex_midi_tuning(data, handled, dryrun);

            return self.api_return(result);
        }

        /* GM or GM2 system on */
        if d(0) == MIDI_SYSEX_UNIV_NON_REALTIME && dev_ok && d(2) == MIDI_SYSEX_GM_ID {
            if let Some(h) = handled.as_deref_mut() {
                *h = true;
            }
            if !dryrun && (d(3) == MIDI_SYSEX_GM_ON || d(3) == MIDI_SYSEX_GM2_ON) {
                self.fluid_synth_api_enter();
                self.bank_select = FLUID_BANK_STYLE_GM;
                let result = self.fluid_synth_system_reset_local();
                if self.verbose != 0 {
                    fluid_log(
                        FLUID_INFO,
                        "Processing SysEX GM / GM2 System ON message, bank selection mode is now gm.",
                    );
                }
                return self.api_return(result);
            }
            return FLUID_OK;
        }

        /* GS DT1 message */
        if d(0) == MIDI_SYSEX_MANUF_ROLAND
            && dev_ok
            && d(2) == MIDI_SYSEX_GS_ID
            && d(3) == MIDI_SYSEX_GS_DT1
        {
            self.fluid_synth_api_enter();
            let result = self.fluid_synth_sysex_gs_dt1(data, handled, dryrun);
            return self.api_return(result);
        }

        /* XG message */
        if d(0) == MIDI_SYSEX_MANUF_YAMAHA && dev_ok && d(2) == MIDI_SYSEX_XG_ID {
            self.fluid_synth_api_enter();
            let result = self.fluid_synth_sysex_xg(data, handled, dryrun);
            if self.verbose != 0 {
                fluid_log(
                    FLUID_INFO,
                    "Processing SysEX XG message, bank selection mode is now xg.",
                );
            }
            return self.api_return(result);
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_sysex_midi_tuning()`: handler for MIDI
    /// tuning SYSEX messages.
    fn fluid_synth_sysex_midi_tuning(
        &mut self,
        data: &[i8],
        mut handled: Option<&mut bool>,
        dryrun: bool,
    ) -> i32 {
        let len = data.len() as i32;
        let mut bank = 0;
        let mut tunedata = [0.0f64; 128];
        let mut keys = [0i32; 128];
        let d = |i: usize| data[i] as i32;

        let realtime = d(0) == MIDI_SYSEX_UNIV_REALTIME;
        let msgid = d(3);

        match msgid {
            MIDI_SYSEX_TUNING_BULK_DUMP_REQ | MIDI_SYSEX_TUNING_BULK_DUMP_REQ_BANK => {
                // (no response buffer: see fluid_synth_sysex())
                if d(3) == MIDI_SYSEX_TUNING_BULK_DUMP_REQ {
                    if len != 5 || d(4) & 0x80 != 0 {
                        return FLUID_OK;
                    }
                } else if len != 6 || d(4) & 0x80 != 0 || d(5) & 0x80 != 0 {
                    return FLUID_OK;
                }

                /* !response */
                return FLUID_OK;
            }

            MIDI_SYSEX_TUNING_NOTE_TUNE | MIDI_SYSEX_TUNING_NOTE_TUNE_BANK => {
                let mut dataptr: usize = 4;

                if msgid == MIDI_SYSEX_TUNING_NOTE_TUNE {
                    if len < 10 || d(4) & 0x80 != 0 || d(5) & 0x80 != 0 || len != d(5) * 4 + 6 {
                        return FLUID_OK;
                    }
                } else {
                    if len < 11
                        || d(4) & 0x80 != 0
                        || d(5) & 0x80 != 0
                        || d(6) & 0x80 != 0
                        || len != d(6) * 4 + 7
                    {
                        return FLUID_OK;
                    }

                    bank = d(dataptr);
                    dataptr += 1;
                }

                if dryrun {
                    if let Some(h) = handled.as_deref_mut() {
                        *h = true;
                    }

                    return FLUID_OK;
                }

                let prog = d(dataptr);
                dataptr += 1;
                let count = d(dataptr);
                dataptr += 1;

                let mut index = 0usize;
                for _ in 0..count {
                    let mut note = d(dataptr);
                    dataptr += 1;

                    if note & 0x80 != 0 {
                        return FLUID_OK;
                    }

                    keys[index] = note;

                    note = d(dataptr);
                    dataptr += 1;
                    let mut frac = d(dataptr);
                    dataptr += 1;
                    let frac2 = d(dataptr);
                    dataptr += 1;

                    if note & 0x80 != 0 || frac & 0x80 != 0 || frac2 & 0x80 != 0 {
                        return FLUID_OK;
                    }

                    frac = (frac << 7) | frac2;

                    /* No change pitch value?  Doesn't really make sense to send that, but.. */
                    if note == 0x7F && frac == 16383 {
                        continue;
                    }

                    tunedata[index] = note as f64 * 100.0 + (frac as f64 * 100.0 / 16384.0);
                    index += 1;
                }

                if index > 0
                    && self.fluid_synth_tune_notes(
                        bank,
                        prog,
                        &keys[..index],
                        &tunedata[..index],
                        realtime,
                    ) == FLUID_FAILED
                {
                    return FLUID_FAILED;
                }

                if let Some(h) = handled.as_deref_mut() {
                    *h = true;
                }
            }

            MIDI_SYSEX_TUNING_OCTAVE_TUNE_1BYTE | MIDI_SYSEX_TUNING_OCTAVE_TUNE_2BYTE => {
                if (msgid == MIDI_SYSEX_TUNING_OCTAVE_TUNE_1BYTE && len != 19)
                    || (msgid == MIDI_SYSEX_TUNING_OCTAVE_TUNE_2BYTE && len != 31)
                {
                    return FLUID_OK;
                }

                if d(4) & 0x80 != 0 || d(5) & 0x80 != 0 || d(6) & 0x80 != 0 {
                    return FLUID_OK;
                }

                if dryrun {
                    if let Some(h) = handled.as_deref_mut() {
                        *h = true;
                    }

                    return FLUID_OK;
                }

                let channels = ((d(4) & 0x03) << 14) | (d(5) << 7) | d(6);

                let mut octave = [0.0f64; 12];
                if msgid == MIDI_SYSEX_TUNING_OCTAVE_TUNE_1BYTE {
                    for (i, t) in octave.iter_mut().enumerate() {
                        let frac = d(i + 7);

                        if frac & 0x80 != 0 {
                            return FLUID_OK;
                        }

                        *t = (frac - 64) as f64;
                    }
                } else {
                    for (i, t) in octave.iter_mut().enumerate() {
                        let frac = d(i * 2 + 7);
                        let frac2 = d(i * 2 + 8);

                        if frac & 0x80 != 0 || frac2 & 0x80 != 0 {
                            return FLUID_OK;
                        }

                        *t = (((frac << 7) | frac2) - 8192) as f64 * (200.0 / 16384.0);
                    }
                }

                if self.fluid_synth_activate_octave_tuning(0, 0, "SYSEX", &octave, realtime)
                    == FLUID_FAILED
                {
                    return FLUID_FAILED;
                }

                if channels != 0 {
                    for i in 0..16 {
                        if channels & (1 << i) != 0 {
                            self.fluid_synth_activate_tuning(i, 0, 0, realtime);
                        }
                    }
                }

                if let Some(h) = handled {
                    *h = true;
                }
            }

            _ => {}
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_sysex_gs_dt1()`: handler for GS DT1
    /// messages.
    fn fluid_synth_sysex_gs_dt1(
        &mut self,
        data: &[i8],
        mut handled: Option<&mut bool>,
        dryrun: bool,
    ) -> i32 {
        let len = data.len() as i32;
        let d = |i: usize| data[i] as i32;
        let mut checksum: i32 = 0;

        if len < 9 {
            // at least one byte of data should be transmitted
            fluid_log(FLUID_INFO, "SysEx GS DT1: message too short, dropping it.");
            return FLUID_FAILED;
        }
        let len_data = len - 8;
        let addr = (d(4).wrapping_shl(16)) | (d(5).wrapping_shl(8)) | d(6);

        for i in 4..(len - 1) as usize {
            checksum = checksum.wrapping_add(d(i));
        }
        checksum = 0x80 - (checksum & 0x7F);
        // An intermediate checksum of 0x80 must be treated as zero! #1578
        if (checksum & 0x7F) != d(len as usize - 1) {
            flog!(
                FLUID_INFO,
                "SysEx GS DT1: dropping message on addr 0x{:x} due to incorrect checksum 0x{:x}. Correct checksum: 0x{:x}",
                addr,
                d(len as usize - 1),
                checksum
            );
            return FLUID_FAILED;
        }

        if addr == 0x40007F {
            // Mode set
            if len_data > 1 || (d(7) != 0 && d(7) != 0x7f) {
                fluid_log(
                    FLUID_INFO,
                    "SysEx GS DT1: dropping invalid mode set message",
                );
                return FLUID_FAILED;
            }
            if let Some(h) = handled.as_deref_mut() {
                *h = true;
            }

            let i = d(7);
            if !dryrun {
                if i == 0 {
                    self.bank_select = FLUID_BANK_STYLE_GS;
                } else {
                    self.bank_select = FLUID_BANK_STYLE_GM;
                }
                return self.fluid_synth_system_reset_local();
            }
            if self.verbose != 0 {
                flog!(
                    FLUID_INFO,
                    "{}SysEX GS DT1: bank selection mode is now {}",
                    if dryrun { "[DRYRUN] " } else { "" },
                    if i == 0 { "GS" } else { "GM" }
                );
            }
            return FLUID_OK;
        }

        if self.bank_select != FLUID_BANK_STYLE_GS {
            return FLUID_OK; // Silently ignore all other messages
        }

        if (addr & 0xFFF0FF) == 0x401015 {
            // Use for rhythm part
            if len_data > 1 || d(7) > 0x02 {
                fluid_log(
                    FLUID_INFO,
                    "SysEx GS DT1: dropping invalid rhythm part message",
                );
                return FLUID_FAILED;
            }
            if let Some(h) = handled {
                *h = true;
            }
            if !dryrun {
                let mut chan = (addr >> 8) & 0x0F;
                //See the Patch Part parameters section in SC-88Pro/8850 owner's manual
                chan = if chan >= 0x0a {
                    chan
                } else if chan == 0 {
                    9
                } else {
                    chan - 1
                };
                let type_ = if d(7) == 0x00 {
                    CHANNEL_TYPE_MELODIC
                } else {
                    CHANNEL_TYPE_DRUM
                };
                self.channel[chan as usize].channel_type = type_;

                flog!(
                    FLUID_DBG,
                    "SysEx GS DT1: setting MIDI channel {} to type {}",
                    chan,
                    self.channel[chan as usize].channel_type
                );
                // Roland synths seem to "remember" the last instrument a channel
                // used in the selected mode. This behavior is not replicated here.
                // Issue 1579: The preset selected for the channel needs to be forcibly changed. Therefore it is not sufficient
                // to send a prog change, as the old bank is still active in the channel.
                // MSGS selects the standard drum kit right after this message (which is equivalent to sending a prog change 0).
                // However, this behavior is insonsistent with the note on site 60:
                // "To select a drum set after setting the part mode, transmit a program change [...]"
                self.fluid_synth_cc_local(chan, ALL_CTRL_OFF);
                self.channel[chan as usize].set_sfont_bank_prog(
                    -1,
                    if type_ == CHANNEL_TYPE_DRUM {
                        DRUM_INST_BANK
                    } else {
                        0
                    },
                    -1,
                );
                self.fluid_synth_program_change(chan, 0);
            }
            return FLUID_OK;
        }

        //silently ignore
        FLUID_OK
    }

    /// Translation of `fluid_synth_sysex_xg()`: handler for XG messages.
    fn fluid_synth_sysex_xg(
        &mut self,
        data: &[i8],
        handled: Option<&mut bool>,
        dryrun: bool,
    ) -> i32 {
        let len = data.len() as i32;
        let d = |i: usize| data[i] as i32;

        if len < 7 {
            // at least one byte of data should be transmitted
            return FLUID_FAILED;
        }
        let len_data = len - 6;
        let addr = (d(3).wrapping_shl(16)) | (d(4).wrapping_shl(8)) | d(5);

        if addr == 0x00007E // Reset
            || addr == 0x00007F
        {
            // Reset to factory
            if len_data > 1 || d(6) != 0 {
                return FLUID_FAILED;
            }
            if let Some(h) = handled {
                *h = true;
            }
            if !dryrun {
                self.bank_select = FLUID_BANK_STYLE_XG;
                return self.fluid_synth_system_reset_local();
            }
            return FLUID_OK;
        }

        /* No other messages handled yet
        if (synth->bank_select != FLUID_BANK_STYLE_XG)
        {
            return FLUID_OK; // Silently ignore all other messages
        }*/

        //silently ignore
        FLUID_OK
    }
}

impl FluidSynthInner {
    /// Translation of `fluid_synth_all_notes_off()`: turn off all voices
    /// that are playing on the given MIDI channel, by putting them into
    /// release phase.
    /// @param chan MIDI channel number (0 to MIDI channel count - 1), (chan=-1 selects all channels)
    pub(crate) fn fluid_synth_all_notes_off(&mut self, chan: i32) -> i32 {
        if chan < -1 {
            return FLUID_FAILED;
        }
        self.fluid_synth_api_enter();

        let result = if chan >= self.midi_channels {
            FLUID_FAILED
        } else {
            /* Allowed (even for channel disabled) as chan = -1 selects all channels */
            self.fluid_synth_all_notes_off_local(chan)
        };

        self.api_return(result)
    }

    /// Translation of `fluid_synth_all_notes_off_LOCAL()`: local synthesis
    /// thread variant of all notes off, (chan=-1 selects all channels)
    pub(crate) fn fluid_synth_all_notes_off_local(&mut self, chan: i32) -> i32 {
        for i in 0..self.polyphony as usize {
            let voice = &self.voice[i];

            if fluid_voice_is_playing(voice)
                && ((-1 == chan) || (chan == fluid_voice_get_channel(voice)))
            {
                let (voice, mut env) = self.split_voice(i);
                fluid_voice_noteoff(voice, &mut env);
            }
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_all_sounds_off()`: immediately stop all
    /// voices on the given MIDI channel (skips release phase).
    /// @param chan MIDI channel number (0 to MIDI channel count - 1), (chan=-1 selects all channels)
    pub(crate) fn fluid_synth_all_sounds_off(&mut self, chan: i32) -> i32 {
        if chan < -1 {
            return FLUID_FAILED;
        }
        self.fluid_synth_api_enter();

        let result = if chan >= self.midi_channels {
            FLUID_FAILED
        } else {
            /* Allowed (even for channel disabled) as chan = -1 selects all channels */
            self.fluid_synth_all_sounds_off_local(chan)
        };

        self.api_return(result)
    }

    /// Translation of `fluid_synth_all_sounds_off_LOCAL()`: local synthesis
    /// thread variant of all sounds off, (chan=-1 selects all channels)
    fn fluid_synth_all_sounds_off_local(&mut self, chan: i32) -> i32 {
        for i in 0..self.polyphony as usize {
            let voice = &self.voice[i];

            if fluid_voice_is_playing(voice)
                && ((-1 == chan) || (chan == fluid_voice_get_channel(voice)))
            {
                fluid_voice_off(&self.voice[i], &mut self.eventhandler);
            }
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_system_reset()`: send MIDI system reset
    /// command (big red 'panic' button), turns off notes, resets
    /// controllers and restores initial basic channel configuration.
    pub(crate) fn fluid_synth_system_reset(&mut self) -> i32 {
        self.fluid_synth_api_enter();
        let result = self.fluid_synth_system_reset_local();
        self.api_return(result)
    }

    /// Translation of `fluid_synth_system_reset_LOCAL()`: local variant of
    /// the system reset command.
    fn fluid_synth_system_reset_local(&mut self) -> i32 {
        if self.verbose != 0 {
            fluid_log(FLUID_INFO, "=== systemreset ===");
        }

        self.fluid_synth_all_sounds_off_local(-1);

        for i in 0..self.midi_channels as usize {
            self.fluid_channel_reset(i);
        }

        /* Basic channel 0, Mode Omni On Poly */
        let n = self.midi_channels;
        self.fluid_synth_set_basic_channel(0, FLUID_CHANNEL_MODE_OMNION_POLY, n);

        fluid_synth_update_mixer(self, RvoiceFunction::MixerResetReverb, 0, 0.0);
        fluid_synth_update_mixer(self, RvoiceFunction::MixerResetChorus, 0, 0.0);

        /* Reset auto portamento mode tracking */
        if self.portamento_time_mode == FLUID_PORTAMENTO_TIME_MODE_AUTO {
            self.portamento_time_has_seen_lsb = false;
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_modulate_voices_LOCAL()`: update voices
    /// on a MIDI channel after a MIDI control change.
    /// @param is_cc Boolean value indicating if ctrl is a CC controller or not
    /// @param ctrl MIDI controller value
    fn fluid_synth_modulate_voices_local(&mut self, chan: i32, is_cc: bool, ctrl: i32) -> i32 {
        for i in 0..self.polyphony as usize {
            if fluid_voice_get_channel(&self.voice[i]) == chan {
                let (voice, mut env) = self.split_voice(i);
                fluid_voice_modulate(voice, &mut env, is_cc, ctrl);
            }
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_modulate_voices_all_LOCAL()`: update
    /// voices on a MIDI channel after all MIDI controllers have been
    /// changed.
    fn fluid_synth_modulate_voices_all_local(&mut self, chan: i32) -> i32 {
        for i in 0..self.polyphony as usize {
            if fluid_voice_get_channel(&self.voice[i]) == chan {
                let (voice, mut env) = self.split_voice(i);
                fluid_voice_modulate_all(voice, &mut env);
            }
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_channel_pressure()`: set the MIDI
    /// channel pressure controller value.
    pub(crate) fn fluid_synth_channel_pressure(&mut self, chan: i32, val: i32) -> i32 {
        if !(0..=127).contains(&val) {
            return FLUID_FAILED;
        }

        if !self.api_entry_chan(chan) {
            return FLUID_FAILED;
        }

        /* Allowed only on MIDI channel enabled */
        if !self.api_chan_enabled(chan) {
            return FLUID_FAILED;
        }

        if self.verbose != 0 {
            flog!(FLUID_INFO, "channelpressure\t{}\t{}", chan, val);
        }

        self.channel[chan as usize].set_channel_pressure(val);
        let result = self.fluid_synth_update_channel_pressure_local(chan);

        self.api_return(result)
    }

    /// Translation of `fluid_synth_update_channel_pressure_LOCAL()`:
    /// updates channel pressure from within synthesis thread.
    fn fluid_synth_update_channel_pressure_local(&mut self, chan: i32) -> i32 {
        self.fluid_synth_modulate_voices_local(chan, false, FLUID_MOD_CHANNELPRESSURE as i32)
    }

    /// Translation of `fluid_synth_key_pressure()`: set the MIDI
    /// polyphonic key pressure controller value.
    pub(crate) fn fluid_synth_key_pressure(&mut self, chan: i32, key: i32, val: i32) -> i32 {
        if !(0..=127).contains(&key) || !(0..=127).contains(&val) {
            return FLUID_FAILED;
        }

        if !self.api_entry_chan(chan) {
            return FLUID_FAILED;
        }

        /* Allowed only on MIDI channel enabled */
        if !self.api_chan_enabled(chan) {
            return FLUID_FAILED;
        }

        if self.verbose != 0 {
            flog!(FLUID_INFO, "keypressure\t{}\t{}\t{}", chan, key, val);
        }

        self.channel[chan as usize].set_key_pressure(key, val);
        let result = self.fluid_synth_update_key_pressure_local(chan, key);

        self.api_return(result)
    }

    /// Translation of `fluid_synth_update_key_pressure_LOCAL()`: updates
    /// key pressure from within synthesis thread.
    fn fluid_synth_update_key_pressure_local(&mut self, chan: i32, key: i32) -> i32 {
        let mut result = FLUID_OK;

        for i in 0..self.polyphony as usize {
            let voice = &self.voice[i];

            if voice.chan as i32 == chan && voice.key as i32 == key {
                let (voice, mut env) = self.split_voice(i);
                result = fluid_voice_modulate(voice, &mut env, false, FLUID_MOD_KEYPRESSURE as i32);

                if result != FLUID_OK {
                    return result;
                }
            }
        }

        result
    }

    /// Translation of `fluid_synth_pitch_bend()`: set the MIDI pitch bend
    /// controller value on a MIDI channel.
    /// @param val MIDI pitch bend value (0-16383 with 8192 being center)
    pub(crate) fn fluid_synth_pitch_bend(&mut self, chan: i32, val: i32) -> i32 {
        if !(0..=16383).contains(&val) {
            return FLUID_FAILED;
        }
        if !self.api_entry_chan(chan) {
            return FLUID_FAILED;
        }

        /* Allowed only on MIDI channel enabled */
        if !self.api_chan_enabled(chan) {
            return FLUID_FAILED;
        }

        if self.verbose != 0 {
            flog!(FLUID_INFO, "pitchb\t\t{}\t{}", chan, val);
        }

        self.channel[chan as usize].set_pitch_bend(val);
        let result = self.fluid_synth_update_pitch_bend_local(chan);

        self.api_return(result)
    }

    /// Translation of `fluid_synth_update_pitch_bend_LOCAL()`: local
    /// synthesis thread variant of pitch bend.
    fn fluid_synth_update_pitch_bend_local(&mut self, chan: i32) -> i32 {
        self.fluid_synth_modulate_voices_local(chan, false, FLUID_MOD_PITCHWHEEL as i32)
    }

    /// Translation of `fluid_synth_update_pitch_wheel_sens_LOCAL()`: local
    /// synthesis thread variant of set pitch wheel sensitivity.
    fn fluid_synth_update_pitch_wheel_sens_local(&mut self, chan: i32) -> i32 {
        self.fluid_synth_modulate_voices_local(chan, false, FLUID_MOD_PITCHWHEELSENS as i32)
    }

    /// Translation of `fluid_synth_set_preset()`: assign a preset to a
    /// MIDI channel.
    fn fluid_synth_set_preset(&mut self, chan: i32, preset: Option<FluidPreset>) -> i32 {
        if !(chan >= 0 && chan < self.midi_channels) {
            return FLUID_FAILED;
        }

        self.channel[chan as usize].set_preset(preset)
    }

    /// Translation of `fluid_synth_find_preset()`: find a preset by bank
    /// and program numbers.
    /// Returns preset pointer or NULL.
    pub(crate) fn fluid_synth_find_preset(
        &self,
        banknum: i32,
        prognum: i32,
    ) -> Option<FluidPreset> {
        for sfont in &self.sfont {
            let preset = sfont.get_preset(banknum - sfont.bankofs, prognum);

            if preset.is_some() {
                return preset;
            }
        }

        None
    }

    /// Translation of `fluid_synth_program_change()`: send a program change
    /// event on a MIDI channel.
    ///
    /// As of 1.1.1 prognum can be set to 128 to unset the preset.  Not documented
    /// since fluid_synth_unset_program() should be used instead.
    pub(crate) fn fluid_synth_program_change(&mut self, chan: i32, prognum: i32) -> i32 {
        let mut preset: Option<FluidPreset> = None;

        if !(0..=128).contains(&prognum) {
            return FLUID_FAILED;
        }
        if !self.api_entry_chan(chan) {
            return FLUID_FAILED;
        }

        /* Allowed only on MIDI channel enabled */
        if !self.api_chan_enabled(chan) {
            return FLUID_FAILED;
        }

        let channel = &self.channel[chan as usize];

        let (_, banknum, _) = channel.get_sfont_bank_prog();

        if self.verbose != 0 {
            flog!(FLUID_INFO, "prog\t\t{}\t{}\t{}", chan, banknum, prognum);
        }

        /* I think this is a hack for MIDI files that do bank changes in GM mode.
         * Proper way to handle this would probably be to ignore bank changes when in
         * GM mode. - JG
         * This is now possible by setting synth.midi-bank-select=gm, but let the hack
         * stay for the time being. - DH
         */
        if prognum != FLUID_UNSET_PROGRAM {
            let mut subst_bank = banknum;
            let mut subst_prog = prognum;

            preset = self.fluid_synth_find_preset(subst_bank, subst_prog);

            /* Fallback to another preset if not found */
            if preset.is_none() {
                /* Percussion: Fallback to preset 0 in percussion bank */
                if channel.channel_type == CHANNEL_TYPE_DRUM {
                    subst_bank = DRUM_INST_BANK;
                    preset = self.fluid_synth_find_preset(subst_bank, subst_prog);
                    if preset.is_none() {
                        subst_prog = 0;
                        preset = self.fluid_synth_find_preset(subst_bank, subst_prog);
                    }
                }
                /* Melodic instrument */
                else {
                    /* Fallback first to bank 0:prognum */
                    subst_bank = 0;
                    preset = self.fluid_synth_find_preset(subst_bank, subst_prog);

                    /* Fallback to first preset in bank 0 (usually piano...) */
                    if preset.is_none() {
                        subst_prog = 0;
                        preset = self.fluid_synth_find_preset(subst_bank, subst_prog);
                    }
                }

                if preset.is_some() {
                    flog!(
                        FLUID_WARN,
                        "Instrument not found on channel {} [bank={} prog={}], substituted [bank={} prog={}]",
                        chan,
                        banknum,
                        prognum,
                        subst_bank,
                        subst_prog
                    );
                } else {
                    flog!(
                        FLUID_WARN,
                        "No preset found on channel {} [bank={} prog={}]",
                        chan,
                        banknum,
                        prognum
                    );
                }
            }
        }

        /* Assign the SoundFont ID and program number to the channel */
        let sfont_id = preset.as_ref().map_or(0, |p| p.get_sfont_id());
        self.channel[chan as usize].set_sfont_bank_prog(sfont_id, -1, prognum);
        let result = self.fluid_synth_set_preset(chan, preset);

        self.api_return(result)
    }

    /// Translation of `fluid_synth_program_reset()`: resend a bank select
    /// and a program change for every channel and assign corresponding
    /// instruments.
    ///
    /// This function is called mainly after a SoundFont has been loaded,
    /// unloaded or reloaded.
    pub(crate) fn fluid_synth_program_reset(&mut self) -> i32 {
        self.fluid_synth_api_enter();

        /* try to set the correct presets */
        for i in 0..self.midi_channels {
            let (_, _, prog) = self.channel[i as usize].get_sfont_bank_prog();
            self.fluid_synth_program_change(i, prog);
        }

        self.api_return(FLUID_OK)
    }

    /// Translation of `fluid_synth_write_float()`: synthesize a block of
    /// floating point audio samples to audio buffers.
    ///
    /// @param len Count of audio frames to synthesize
    /// @param lout Array of floats to store left channel of audio
    /// @param loff Offset index in 'lout' for first sample
    /// @param lincr Increment between samples stored to 'lout'
    /// @param rout Array of floats to store right channel of audio, or None
    ///   to store it to 'lout' as well (as upstream with lout == rout)
    /// @param roff Offset index in 'rout' for first sample
    /// @param rincr Increment between samples stored to 'rout'
    /// @return #FLUID_OK on success, #FLUID_FAILED otherwise
    ///
    /// Useful for storing interleaved stereo (lout = rout, loff = 0, roff = 1,
    /// lincr = 2, rincr = 2).
    ///
    /// @note Reverb and Chorus are mixed to \c lout resp. \c rout.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fluid_synth_write_float(
        &mut self,
        len: i32,
        lout: &mut [f32],
        loff: i32,
        lincr: i32,
        rout: Option<&mut [f32]>,
        roff: i32,
        rincr: i32,
    ) -> i32 {
        self.fluid_synth_write_float_channels_local(len, lout, loff, lincr, rout, roff, rincr)
    }

    /// Translation of `fluid_synth_write_float_channels_LOCAL()` for the
    /// two channels of `fluid_synth_write_float()` (and its block render
    /// function, `fluid_synth_render_blocks()`).
    #[allow(clippy::too_many_arguments)]
    fn fluid_synth_write_float_channels_local(
        &mut self,
        len: i32,
        lout: &mut [f32],
        loff: i32,
        lincr: i32,
        mut rout: Option<&mut [f32]>,
        roff: i32,
        rincr: i32,
    ) -> i32 {
        /* start average cpu load probe */
        let time = crate::utils::sys::fluid_utime();

        /* check parameters */
        if len < 0 {
            return FLUID_FAILED;
        }
        if len == 0 {
            return FLUID_OK; // to avoid raising FE_DIVBYZERO below
        }

        /* check for valid channel_count: must be multiple of 2 and
        channel_count/2 must not exceed the number of internal mixer buffers
        (synth->audio_groups)
        */
        let bufs_in_count = 1; /* number of stereo input buffers */
        if bufs_in_count > self.audio_groups {
            return FLUID_FAILED;
        }

        // (the C pointers write wherever the offsets and increments take
        // them; here they must stay inside the buffers)
        let fits = |buf_len: usize, off: i32, incr: i32| -> bool {
            let last = off as i64 + (len as i64 - 1) * incr as i64;
            off >= 0 && last >= 0 && (off as i64) < buf_len as i64 && last < buf_len as i64
        };
        let rlen = rout.as_ref().map_or(lout.len(), |r| r.len());
        if !fits(lout.len(), loff, lincr) || !fits(rlen, roff, rincr) {
            return FLUID_FAILED;
        }

        /* initialize output channels buffers on first sample position */
        let mut lpos = loff as i64;
        let mut rpos = roff as i64;

        /* Conversely to fluid_synth_process(),
        we want rendered audio effect mixed in internal audio dry buffers.
        TRUE instructs the mixer that internal audio effects will be mixed in internal
        audio dry buffers.
        */
        self.eventhandler.mixer.set_mix_fx(1);

        let mut size = len;

        /* synth->cur indicates if available samples are still in internal mixer buffer */
        let mut cur = self.cur; /* get previous sample position in internal buffer (due to prvious call) */

        loop {
            /* fill up the buffers as needed */
            if cur >= self.curmax {
                /* render audio (dry and effect) to internal dry buffers */
                /* always render full blocks multiple of FLUID_BUFSIZE */
                let blocksleft = (size + FLUID_BUFSIZE as i32 - 1) / FLUID_BUFSIZE as i32;
                self.curmax = FLUID_BUFSIZE as i32 * self.fluid_synth_render_blocks(blocksleft);

                cur = 0;
            }

            /* calculate amount of available samples */
            let mut n = self.curmax - cur;

            /* keep track of emitted samples */
            if n > size {
                n = size;
            }

            size -= n;

            /* get first internal mixer audio dry buffer's pointer (left and right channel) */
            /* update pointers to current position */
            let left_in = &self.eventhandler.mixer.buffers.left_buf;
            let right_in = &self.eventhandler.mixer.buffers.right_buf;
            let base = (cur + n) as i64;

            /* set final cursor position */
            cur += n;

            /* reverse index */
            let mut k = -(n as i64);

            while k < 0 {
                let mut i = bufs_in_count;
                loop {
                    /* input sample index in stereo buffer i */
                    i -= 1;
                    let in_idx = (i as i64
                        * (FLUID_BUFSIZE * FLUID_MIXER_MAX_BUFFERS_DEFAULT) as i64
                        + base
                        + k) as usize;

                    /* write left input sample to channel sample */
                    lout[lpos as usize] = left_in[in_idx] as f32;

                    /* write right input sample to next channel sample */
                    let r = right_in[in_idx] as f32;
                    match rout.as_deref_mut() {
                        Some(rout) => rout[rpos as usize] = r,
                        None => lout[rpos as usize] = r,
                    }

                    /* advance output pointers */
                    lpos += lincr as i64;
                    rpos += rincr as i64;

                    if i == 0 {
                        break;
                    }
                }
                k += 1;
            }

            if size == 0 {
                break;
            }
        }

        self.cur = cur; /* save current sample position. It will be used on next call */

        /* save average cpu load, use by API for real time cpu load meter */
        let time = crate::utils::sys::fluid_utime() - time;
        self.cpu_load =
            (0.5 * (self.cpu_load as f64 + time * self.sample_rate / len as f64 / 10000.0)) as f32;

        FLUID_OK
    }

    /// Translation of `fluid_synth_check_finished_voices()`.
    fn fluid_synth_check_finished_voices(&mut self) {
        while let Some(fv) = self.eventhandler.get_finished_voice() {
            for j in 0..self.polyphony as usize {
                if self.voice[j].rvoice == fv {
                    let voice = &mut self.voice[j];
                    voice.can_access_rvoice = true; /* fluid_voice_unlock_rvoice() */
                    fluid_voice_stop(voice, &mut self.active_voice_count);
                    break;
                } else if self.voice[j].overflow_rvoice == fv {
                    /* Unlock the overflow_rvoice of the voice.
                    Decrement the reference count of the sample owned by this
                    rvoice.
                    */
                    fluid_voice_overflow_rvoice_finished(&mut self.voice[j]);

                    /* Decrement synth active voice count. Must not be incorporated
                    in fluid_voice_overflow_rvoice_finished() because
                    fluid_voice_overflow_rvoice_finished() is called also
                    at synth destruction and in this case the variable should be
                    accessed via voice->channel->synth->active_voice_count.
                    And for certain voices which are not playing, the field
                    voice->channel is NULL.
                    */
                    self.active_voice_count -= 1;
                    break;
                }
            }
        }
    }

    /// Translation of `fluid_synth_process_event_queue()`: process all
    /// waiting events in the rvoice queue.
    pub(crate) fn fluid_synth_process_event_queue(&mut self) {
        self.eventhandler.dispatch_all();
    }

    /// Translation of `fluid_synth_render_blocks()`: process blocks
    /// (FLUID_BUFSIZE) of audio.
    /// @return number of blocks rendered. Might (often) return less than requested
    fn fluid_synth_render_blocks(&mut self, mut blockcount: i32) -> i32 {
        self.eventhandler.dispatch_all();

        /* do not render more blocks than we can store internally */
        let maxblocks = self.eventhandler.mixer.get_bufcount();

        if blockcount > maxblocks {
            blockcount = maxblocks;
        }

        for i in 0..blockcount {
            self.fluid_sample_timer_process();
            self.add_ticks(FLUID_BUFSIZE as i32);

            /* If events have been queued waiting for fluid_rvoice_eventhandler_dispatch_all()
             * (should only happen with parallel render) stop processing and go for rendering
             */
            if self.eventhandler.dispatch_count() != 0 {
                // Something has happened, we can't process more
                blockcount = i + 1;
                break;
            }
        }

        let eh = &mut self.eventhandler;
        blockcount = fluid_rvoice_mixer_render(&mut eh.mixer, &mut eh.finished_voices, blockcount);

        blockcount
    }

    /// Translation of `fluid_synth_free_voice_by_kill_LOCAL()`: selects a
    /// voice for killing.
    fn fluid_synth_free_voice_by_kill_local(&mut self) -> Option<usize> {
        let mut best_prio: f32 = OVERFLOW_PRIO_CANNOT_KILL - 1.0;
        let mut best_voice_index: i32 = -1;
        let ticks = self.get_ticks();

        for i in 0..self.polyphony as usize {
            let voice = &self.voice[i];

            /* safeguard against an available voice. */
            if available(voice) {
                return Some(i);
            }

            let channel = &self.channel[voice.channel.expect("playing voice with a channel")];
            let this_voice_prio =
                fluid_voice_get_overflow_prio(voice, channel, &self.overflow, ticks);

            /* check if this voice has less priority than the previous candidate. */
            if this_voice_prio < best_prio {
                best_voice_index = i as i32;
                best_prio = this_voice_prio;
            }
        }

        if best_voice_index < 0 {
            return None;
        }

        let v = best_voice_index as usize;
        let voice = &self.voice[v];
        flog!(
            FLUID_DBG,
            "Killing voice {}, index {}, chan {}, key {} ",
            voice.id,
            best_voice_index,
            fluid_voice_get_channel(voice),
            fluid_voice_get_key(voice)
        );
        fluid_voice_off(&self.voice[v], &mut self.eventhandler);

        Some(v)
    }

    /// Translation of `fluid_synth_alloc_voice_LOCAL()`: allocate a
    /// synthesis voice (its index).
    pub(crate) fn fluid_synth_alloc_voice_local(
        &mut self,
        sample: &Arc<Sample>,
        chan: i32,
        key: i32,
        vel: i32,
        zone_range: Option<Arc<FluidZoneRange>>,
    ) -> Option<usize> {
        let mut voice: Option<usize> = None;

        /* check if there's an available synthesis process */
        for i in 0..self.polyphony as usize {
            if available(&self.voice[i]) {
                voice = Some(i);
                break;
            }
        }

        /* No success yet? Then stop a running voice. */
        if voice.is_none() {
            fluid_log(FLUID_DBG, "Polyphony exceeded, trying to kill a voice");
            voice = self.fluid_synth_free_voice_by_kill_local();
        }

        let Some(v) = voice else {
            flog!(
                FLUID_WARN,
                "Failed to allocate a synthesis process. (chan={},key={})",
                chan,
                key
            );
            return None;
        };

        let ticks = self.get_ticks();

        if self.verbose != 0 {
            let mut k = 0;

            for i in 0..self.polyphony as usize {
                if !available(&self.voice[i]) {
                    k += 1;
                }
            }

            flog!(
                FLUID_INFO,
                "noteon\t{}\t{}\t{}\t{:05}\t{:.3}\t{:.3}\t{:.3}\t{}",
                chan,
                key,
                vel,
                self.storeid,
                (ticks as f32 as f64 / self.sample_rate),
                (fluid_curtime().wrapping_sub(self.start)) as f32 / 1000.0f32,
                0.0f32,
                k
            );
        }

        let storeid = self.storeid;
        let gain = self.gain as FluidReal;
        let c = chan as usize;
        {
            // (fluid_voice_init() with the voice's environment and the new
            // channel)
            let voice = &mut self.voice[v];
            let prev_channel = voice.channel.map(|pc| &self.channel[pc]);
            let mut env = VoiceEnv {
                eh: &mut self.eventhandler,
                channel: prev_channel,
                active_voice_count: &mut self.active_voice_count,
                audio_groups: self.audio_groups,
                effects_groups: self.effects_groups,
                effects_channels: self.effects_channels,
                fromkey_portamento: self.fromkey_portamento,
                portamento_time_mode: self.portamento_time_mode,
                portamento_time_has_seen_lsb: self.portamento_time_has_seen_lsb,
                min_note_length_ticks: self.min_note_length_ticks,
            };
            if fluid_voice_init(
                voice,
                &mut env,
                sample,
                zone_range,
                c,
                &self.channel[c],
                key,
                vel,
                storeid,
                ticks,
                gain,
            ) != FLUID_OK
            {
                fluid_log(FLUID_WARN, "Failed to initialize voice");
                return None;
            }
        }

        /* add the default modulators to the synthesis process. */
        /* custom_breath2att_modulator is not a default modulator specified in SF
        it is intended to replace default_vel2att_mod for this channel on demand using
        API fluid_synth_set_breath_mode() or shell command setbreathmode for this channel.
        */
        {
            let channel = &self.channel[c];
            let mono = channel.is_playing_mono();
            let default_mod: &[FluidMod] = match &sample.default_modulators {
                Some(mods) => mods,
                None => &self.default_mod,
            };

            let voice = &mut self.voice[v];
            for default_mod in default_mod {
                if
                /* See if default_mod is the velocity_to_attenuation modulator */
                fluid_mod_test_identity(default_mod, &DEFAULT_VEL2ATT_MOD) &&
                    // See if a replacement by custom_breath2att_modulator has been demanded
                    // for this channel
                    ((!mono && (channel.mode & FLUID_CHANNEL_BREATH_POLY) != 0)
                        || (mono && (channel.mode & FLUID_CHANNEL_BREATH_MONO) != 0))
                {
                    // Replacement of default_vel2att modulator by custom_breath2att_modulator
                    fluid_voice_add_mod_local(
                        voice,
                        &CUSTOM_BREATH2ATT_MOD,
                        FLUID_VOICE_DEFAULT,
                        0,
                    );
                } else {
                    fluid_voice_add_mod_local(voice, default_mod, FLUID_VOICE_DEFAULT, 0);
                }

                // Next default modulator to add to the voice
            }
        }

        Some(v)
    }

    /// Translation of `fluid_synth_kill_by_exclusive_class_LOCAL()`: kill
    /// all voices on a given channel, which have the same exclusive class
    /// generator as new_voice.
    fn fluid_synth_kill_by_exclusive_class_local(&mut self, new_voice: usize) {
        let excl_class = fluid_voice_gen_value(&self.voice[new_voice], GEN_EXCLUSIVECLASS) as i32;

        /* Excl. class 0: No exclusive class */
        if excl_class == 0 {
            return;
        }

        let new_chan = fluid_voice_get_channel(&self.voice[new_voice]);
        let new_id = self.voice[new_voice].id;

        /* Kill all notes on the same channel with the same exclusive class */
        for i in 0..self.polyphony as usize {
            let existing_voice = &self.voice[i];

            /* If voice is playing, on the same channel, has same exclusive
             * class and is not part of the same noteon event (voice group), then kill it */

            if fluid_voice_is_playing(existing_voice)
                && fluid_voice_get_channel(existing_voice) == new_chan
                && fluid_voice_gen_value(existing_voice, GEN_EXCLUSIVECLASS)
                    == excl_class as FluidReal
                && existing_voice.id != new_id
            {
                let (voice, mut env) = self.split_voice(i);
                fluid_voice_kill_excl(voice, &mut env);
            }
        }
    }

    /// Translation of `fluid_synth_start_voice()`: activate a voice
    /// previously allocated with fluid_synth_alloc_voice().
    ///
    /// This function is called by a SoundFont's preset in response to a noteon
    /// event.  Exclusive classes are processed here.
    pub(crate) fn fluid_synth_start_voice(&mut self, voice: usize) {
        self.fluid_synth_api_enter();

        /* Find the exclusive class of this voice. If set, kill all voices
         * that match the exclusive class and are younger than the first
         * voice process created by this noteon event. */
        self.fluid_synth_kill_by_exclusive_class_local(voice);

        let (v, mut env) = self.split_voice(voice);
        fluid_voice_start(v, &mut env); /* Start the new voice */
        v.can_access_rvoice = false; /* fluid_voice_lock_rvoice() */
        let rvoice = v.rvoice;
        self.eventhandler.add_rvoice(rvoice);
        self.fluid_synth_api_exit();
    }

    /// Translation of `fluid_synth_add_sfloader()`: add a SoundFont loader
    /// to the synth.
    ///
    /// @note Should only be called before any SoundFont files are loaded.
    pub(crate) fn fluid_synth_add_sfloader(&mut self, loader: SfLoader) {
        self.fluid_synth_api_enter();

        /* Test if sfont is already loaded */
        if self.sfont.is_empty() {
            self.loaders.insert(0, loader);
        }

        self.fluid_synth_api_exit();
    }

    /// Translation of `fluid_synth_sfload()`: load a SoundFont file.
    ///
    /// The @p filename is passed onto and interpreted by the SoundFont loaders.
    /// On success, the newly loaded SoundFont will be put on top of the SoundFont
    /// stack. Presets are searched starting from the SoundFont on the
    /// top of the stack, working the way down the stack until a preset is found.
    ///
    /// If the SoundFont is structural defect, it will be rejected and the function will fail.
    ///
    /// @param reset_presets TRUE to re-assign presets for all MIDI channels (equivalent to calling fluid_synth_program_reset())
    /// @return SoundFont ID on success, #FLUID_FAILED on error
    pub(crate) fn fluid_synth_sfload(&mut self, filename: &str, reset_presets: bool) -> i32 {
        self.fluid_synth_api_enter();

        let mut sfont_id = self.sfont_id;

        sfont_id = sfont_id.wrapping_add(1);
        if sfont_id != FLUID_FAILED {
            /* MT NOTE: Loaders list should not change. */

            for l in 0..self.loaders.len() {
                let sfont = self.loaders[l].load(filename);

                if let Some(mut sfont) = sfont {
                    self.sfont_id = sfont_id;
                    sfont.id = sfont_id;

                    self.sfont.insert(0, sfont); /* prepend to list */

                    /* reset the presets for all channels if requested */
                    if reset_presets {
                        self.fluid_synth_program_reset();
                    }

                    return self.api_return(sfont_id);
                }
            }
        }

        flog!(FLUID_ERR, "Failed to load SoundFont \"{}\"", filename);
        self.api_return(FLUID_FAILED)
    }

    /// Translation of `fluid_synth_reverb_on()`: enable or disable reverb
    /// on one fx group unit.
    /// @param fx_group Index of the fx group.
    ///  Must be in the range <code>-1 to (fluid_synth_count_effects_groups()-1)</code>. If -1 the
    ///  parameter will be applied to all fx groups.
    /// @param on TRUE to enable reverb, FALSE to disable
    pub(crate) fn fluid_synth_reverb_on(&mut self, fx_group: i32, on: i32) -> i32 {
        let mut param = params();

        self.fluid_synth_api_enter();

        if fx_group < -1 || fx_group >= self.effects_groups {
            return self.api_return(FLUID_FAILED);
        }

        if fx_group < 0 {
            self.with_reverb = (on != 0) as i32;
        }

        param[0].i = fx_group;
        param[1].i = on;
        let ret = push_result(self.eventhandler.push(
            RvoiceFunction::MixerReverbEnable,
            RvoiceObject::Mixer,
            param,
        ));

        self.api_return(ret)
    }

    /// Translation of `fluid_synth_set_reverb_full()`.
    pub(crate) fn fluid_synth_set_reverb_full(
        &mut self,
        fx_group: i32,
        set: i32,
        values: &[f64; FLUID_REVERB_PARAM_LAST],
    ) -> i32 {
        let mut param = params();

        /* if non of the flags is set, fail */
        if set & FLUID_REVMODEL_SET_ALL == 0 {
            return FLUID_FAILED;
        }

        /* fx group shadow values are set here so that they will be returned if queried */
        self.eventhandler
            .mixer
            .set_reverb_full(fx_group, set, values);

        /* Synth shadow values are set here so that they will be returned if queried */
        if fx_group < 0 {
            for i in 0..FLUID_REVERB_PARAM_LAST {
                if set & fluid_revparam_to_setflag(i) != 0 {
                    self.reverb_param[i] = values[i];
                }
            }
        }

        param[0].i = fx_group;
        param[1].i = set;
        param[2].real = values[FLUID_REVERB_ROOMSIZE];
        param[3].real = values[FLUID_REVERB_DAMP];
        param[4].real = values[FLUID_REVERB_WIDTH];
        param[5].real = values[FLUID_REVERB_LEVEL];
        /* finally enqueue an rvoice event to the mixer to actual update reverb */
        push_result(self.eventhandler.push(
            RvoiceFunction::MixerSetReverbParams,
            RvoiceObject::Mixer,
            param,
        ))
    }

    /// Translation of `fluid_synth_chorus_on()`: enable or disable chorus
    /// on one or all groups.
    /// @param fx_group Index of the fx group.
    ///  Must be in the range <code>-1 to (fluid_synth_count_effects_groups()-1)</code>. If -1 the
    ///  parameter will be applied to all fx groups.
    /// @param on TRUE to enable chorus, FALSE to disable
    pub(crate) fn fluid_synth_chorus_on(&mut self, fx_group: i32, on: i32) -> i32 {
        let mut param = params();

        self.fluid_synth_api_enter();

        if fx_group < -1 || fx_group >= self.effects_groups {
            return self.api_return(FLUID_FAILED);
        }

        if fx_group < 0 {
            self.with_chorus = (on != 0) as i32;
        }

        param[0].i = fx_group;
        param[1].i = on;
        let ret = push_result(self.eventhandler.push(
            RvoiceFunction::MixerChorusEnable,
            RvoiceObject::Mixer,
            param,
        ));

        self.api_return(ret)
    }

    /// Translation of `fluid_synth_set_chorus_full()`.
    pub(crate) fn fluid_synth_set_chorus_full(
        &mut self,
        fx_group: i32,
        set: i32,
        values: &[f64; FLUID_CHORUS_PARAM_LAST],
    ) -> i32 {
        let mut param = params();

        /* if non of the flags is set, fail */
        if set & FLUID_CHORUS_SET_ALL == 0 {
            return FLUID_FAILED;
        }

        /* fx group shadow values are set here so that they will be returned if queried */
        self.eventhandler
            .mixer
            .set_chorus_full(fx_group, set, values);

        /* Synth shadow values are set here so that they will be returned if queried */
        if fx_group < 0 {
            for i in 0..FLUID_CHORUS_PARAM_LAST {
                if set & fluid_chorparam_to_setflag(i) != 0 {
                    self.chorus_param[i] = values[i];
                }
            }
        }

        param[0].i = fx_group;
        param[1].i = set;
        param[2].i = values[FLUID_CHORUS_NR] as i32;
        param[3].real = values[FLUID_CHORUS_LEVEL];
        param[4].real = values[FLUID_CHORUS_SPEED];
        param[5].real = values[FLUID_CHORUS_DEPTH];
        param[6].i = values[FLUID_CHORUS_TYPE] as i32;
        push_result(self.eventhandler.push(
            RvoiceFunction::MixerSetChorusParams,
            RvoiceObject::Mixer,
            param,
        ))
    }

    /// Translation of `fluid_synth_release_voice_on_same_note_LOCAL()`.
    ///
    /// If the same note is hit twice on the same channel, then the older
    /// voice process is advanced to the release stage.  Using a mechanical
    /// MIDI controller, the only way this can happen is when the sustain
    /// pedal is held.  In this case the behaviour implemented here is
    /// natural for many instruments.  Note: One noteon event can trigger
    /// several voice processes, for example a stereo sample.  Don't
    /// release those...
    pub(crate) fn fluid_synth_release_voice_on_same_note_local(&mut self, chan: i32, key: i32) {
        /* storeid is a parameter for fluid_voice_init() */
        self.storeid = self.noteid;
        self.noteid = self.noteid.wrapping_add(1);

        /* for "monophonic playing" key is the previous sustained note
        if it exists (0 to 127) or INVALID_NOTE otherwise */
        if key == INVALID_NOTE as i32 {
            return;
        }

        for i in 0..self.polyphony as usize {
            let voice = &self.voice[i];

            if fluid_voice_is_playing(voice)
                && (fluid_voice_get_channel(voice) == chan)
                && (fluid_voice_get_key(voice) == key)
                && (voice.id != self.noteid)
            {
                let type_ = self.channel[chan as usize].channel_type;
                let note_cut_mode = self.msgs_note_cut_mode;

                /* Id of voices that was sustained by sostenuto */
                if fluid_voice_is_sostenuto(voice) {
                    self.storeid = voice.id;
                }

                if note_cut_mode == FLUID_MSGS_DISABLED
                    || (note_cut_mode == FLUID_MSGS_DRUM_CUT && type_ == CHANNEL_TYPE_MELODIC)
                {
                    /* Force the voice into release stage except if pedaling (sostenuto or sustain) is active.
                     * This gives a more realistic sound to pianos and possibly other instruments (see PR #905). */
                    let (voice, mut env) = self.split_voice(i);
                    fluid_voice_noteoff(voice, &mut env);
                } else {
                    // i.e. if((note_cut_mode == FLUID_MSGS_ALL_CUT) || (note_cut_mode == FLUID_MSGS_DRUM_CUT && type == CHANNEL_TYPE_DRUM))
                    /* release the voice, this should make riding hi-hats or snares sound more realistic (Discussion #1196)
                     * Note however, that this is not a SF2 compliant behavior, see #1466 */
                    fluid_voice_off(&self.voice[i], &mut self.eventhandler);
                }
            }
        }
    }

    /// Translation of `fluid_synth_get_tuning()`: get tuning for a given
    /// bank:program.
    fn fluid_synth_get_tuning(&self, bank: i32, prog: i32) -> Option<Arc<FluidTuning>> {
        self.tuning
            .as_ref()?
            .get(bank as usize)?
            .as_ref()?
            .get(prog as usize)?
            .clone()
    }

    /// Translation of `fluid_synth_replace_tuning_LOCK()`: replace tuning on
    /// a given bank:program (need not already exist).
    /// Synth mutex should already be locked by caller.
    fn fluid_synth_replace_tuning_lock(
        &mut self,
        tuning: Arc<FluidTuning>,
        bank: i32,
        prog: i32,
        apply: bool,
    ) -> i32 {
        let table = self.tuning.get_or_insert_with(|| vec![None; 128]);

        let bank_table = table[bank as usize].get_or_insert_with(|| vec![None; 128]);

        let old_tuning = bank_table[prog as usize].replace(tuning.clone());

        if let Some(old_tuning) = old_tuning {
            // (fluid_tuning_unref(): the old tuning is deleted unless
            // channels still use it)
            if Arc::strong_count(&old_tuning) > 1 {
                /* -- unref old tuning */
                /* Replace old tuning if present */
                self.fluid_synth_replace_tuning_local(&old_tuning, Some(tuning), apply);
            }
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_replace_tuning_LOCAL()`: replace a
    /// tuning with a new one in all MIDI channels.  new_tuning can be
    /// NULL, in which case channels are reset to default equal tempered
    /// scale.
    fn fluid_synth_replace_tuning_local(
        &mut self,
        old_tuning: &Arc<FluidTuning>,
        new_tuning: Option<Arc<FluidTuning>>,
        apply: bool,
    ) {
        for i in 0..self.midi_channels as usize {
            let channel = &mut self.channel[i];

            if channel
                .get_tuning()
                .is_some_and(|t| Arc::ptr_eq(t, old_tuning))
            {
                /* ++ ref new tuning for channel */
                channel.set_tuning(new_tuning.clone());

                if apply {
                    self.fluid_synth_update_voice_tuning_local(i);
                }
            }
        }

        /* Send unref old tuning event if any unrefs */
        // (the Arcs)
    }

    /// Translation of `fluid_synth_update_voice_tuning_LOCAL()`: update
    /// voice tunings in realtime.
    fn fluid_synth_update_voice_tuning_local(&mut self, channel: usize) {
        for i in 0..self.polyphony as usize {
            let voice = &self.voice[i];

            if fluid_voice_is_on(voice) && (voice.channel == Some(channel)) {
                let (voice, mut env) = self.split_voice(i);
                fluid_voice_calculate_gen_pitch(voice, env.channel.expect("voice with a channel"));
                fluid_voice_update_param(voice, &mut env, GEN_PITCH);
            }
        }
    }

    /// Translation of `fluid_synth_activate_octave_tuning()`: activate an
    /// octave tuning on every octave in the MIDI note scale.
    /// @param bank Tuning bank number (0-127), not related to MIDI instrument bank
    /// @param prog Tuning preset number (0-127), not related to MIDI instrument program
    /// @param name Label name for this tuning
    /// @param pitch Array of pitch values (length of 12 for each note of an octave
    ///   starting at note C, values are number of offset cents to add to the normal
    ///   tuning amount)
    /// @param apply TRUE to apply new tuning in realtime to existing notes which
    ///   are using the replaced tuning (if any), FALSE otherwise
    fn fluid_synth_activate_octave_tuning(
        &mut self,
        bank: i32,
        prog: i32,
        name: &str,
        pitch: &[f64; 12],
        apply: bool,
    ) -> i32 {
        if !(0..128).contains(&bank) || !(0..128).contains(&prog) {
            return FLUID_FAILED;
        }

        self.fluid_synth_api_enter();
        let mut tuning = FluidTuning::new(Some(name), bank, prog);

        tuning.set_octave(pitch);
        let retval = self.fluid_synth_replace_tuning_lock(Arc::new(tuning), bank, prog, apply);

        self.api_return(retval)
    }

    /// Translation of `fluid_synth_tune_notes()`: set tuning values for one
    /// or more MIDI notes for an existing tuning.
    /// @param bank Tuning bank number (0-127), not related to MIDI instrument bank
    /// @param prog Tuning preset number (0-127), not related to MIDI instrument program
    /// @param key Array of MIDI key numbers (length of 'len', values 0-127)
    /// @param pitch Array of pitch values (length of 'len', values are number of
    ///   cents from MIDI note 0)
    /// @param apply TRUE to apply tuning change in realtime to existing notes using
    ///   the specified tuning, FALSE otherwise
    ///
    /// @note Prior to version 1.1.0 it was an error to specify a tuning that didn't
    /// already exist. Starting with 1.1.0, the default equal tempered scale will be
    /// used as a basis, if no tuning exists for the given bank and prog.
    fn fluid_synth_tune_notes(
        &mut self,
        bank: i32,
        prog: i32,
        key: &[i32],
        pitch: &[f64],
        apply: bool,
    ) -> i32 {
        if !(0..128).contains(&bank) || !(0..128).contains(&prog) || key.is_empty() {
            return FLUID_FAILED;
        }

        self.fluid_synth_api_enter();

        let old_tuning = self.fluid_synth_get_tuning(bank, prog);

        let mut new_tuning = match old_tuning {
            Some(old_tuning) => old_tuning.duplicate(),
            None => FluidTuning::new(Some("Unnamed"), bank, prog),
        };

        for (&k, &p) in key.iter().zip(pitch) {
            new_tuning.set_pitch(k, p);
        }

        let retval = self.fluid_synth_replace_tuning_lock(Arc::new(new_tuning), bank, prog, apply);

        self.api_return(retval)
    }

    /// Translation of `fluid_synth_activate_tuning()`: activate a tuning
    /// scale on a MIDI channel.
    /// @param bank Tuning bank number (0-127), not related to MIDI instrument bank
    /// @param prog Tuning preset number (0-127), not related to MIDI instrument program
    /// @param apply TRUE to apply tuning change to active notes, FALSE otherwise
    ///
    /// @note A default equal tempered scale will be created, if no tuning exists
    /// on the given bank and prog.
    fn fluid_synth_activate_tuning(&mut self, chan: i32, bank: i32, prog: i32, apply: bool) -> i32 {
        if !(0..128).contains(&bank) || !(0..128).contains(&prog) {
            return FLUID_FAILED;
        }

        if !self.api_entry_chan(chan) {
            return FLUID_FAILED;
        }

        let mut tuning = self.fluid_synth_get_tuning(bank, prog);

        /* If no tuning exists, create a new default tuning.  We do this, so that
         * it can be replaced later, if any changes are made. */
        if tuning.is_none() {
            let t = Arc::new(FluidTuning::new(Some("Unnamed"), bank, prog));
            self.fluid_synth_replace_tuning_lock(t.clone(), bank, prog, false);
            tuning = Some(t);
        }

        let Some(tuning) = tuning else {
            return self.api_return(FLUID_FAILED);
        };

        let retval = self.fluid_synth_set_tuning_local(chan, Some(tuning), apply);

        self.api_return(retval)
    }

    /// Translation of `fluid_synth_set_tuning_LOCAL()`: local synthesis
    /// thread set tuning function (takes over tuning reference).
    fn fluid_synth_set_tuning_local(
        &mut self,
        chan: i32,
        tuning: Option<Arc<FluidTuning>>,
        apply: bool,
    ) -> i32 {
        let channel = &mut self.channel[chan as usize];

        channel.set_tuning(tuning); /* !! Takes over callers reference */

        if apply {
            self.fluid_synth_update_voice_tuning_local(chan as usize);
        }

        /* Send unref old tuning event */
        // (the Arcs)

        FLUID_OK
    }

    /// Translation of `fluid_synth_set_gen_LOCAL()`: synthesis thread local
    /// set gen function.
    fn fluid_synth_set_gen_local(&mut self, chan: i32, param: usize, value: f32) {
        self.channel[chan as usize].set_gen(param, value as FluidReal);

        for i in 0..self.polyphony as usize {
            if fluid_voice_get_channel(&self.voice[i]) == chan {
                let (voice, mut env) = self.split_voice(i);
                fluid_voice_set_param(voice, &mut env, param, value as FluidReal);
            }
        }
    }

    /// Translation of `fluid_synth_process_awe32_nrpn_LOCAL()`.
    ///
    /// This implementation is based on "Frequently Asked Questions for SB AWE32" http://archive.gamedev.net/archive/reference/articles/article445.html
    /// as well as on the "SB AWE32 Developer's Information Pack" https://github.com/user-attachments/files/15757220/adip301.pdf
    ///
    /// @param gen the AWE32 effect or generator to manipulate
    /// @param data the composed value of DATA_MSB and DATA_LSB
    fn fluid_synth_process_awe32_nrpn_local(
        &mut self,
        chan: i32,
        gen: i32,
        mut data: i32,
        mut data_lsb: i32,
    ) {
        static AWE32_TO_SF2_GEN: [usize; 27] = [
            // assuming LFO1 maps to MODLFO and LFO2 maps to VIBLFO
            // observe how nicely most of the AWE32 generators here match up with the order of SF2 generators in fluid_gen_type
            GEN_MODLFODELAY,      /* Modulation LFO delay */
            GEN_MODLFOFREQ,       /* Modulation LFO frequency */
            GEN_VIBLFODELAY,      /* Vibrato LFO delay */
            GEN_VIBLFOFREQ,       /* Vibrato LFO frequency */
            GEN_MODENVDELAY,      /* Modulation envelope delay */
            GEN_MODENVATTACK,     /* Modulation envelope attack */
            GEN_MODENVHOLD,       /* Modulation envelope hold */
            GEN_MODENVDECAY,      /* Modulation envelope decay */
            GEN_MODENVSUSTAIN,    /* Modulation envelope sustain */
            GEN_MODENVRELEASE,    /* Modulation envelope release */
            GEN_VOLENVDELAY,      /* Volume envelope delay */
            GEN_VOLENVATTACK,     /* Volume envelope attack */
            GEN_VOLENVHOLD,       /* Volume envelope hold */
            GEN_VOLENVDECAY,      /* Volume envelope decay */
            GEN_VOLENVSUSTAIN,    /* Volume envelope sustain */
            GEN_VOLENVRELEASE,    /* Volume envelope release */
            GEN_PITCH,            /* Initial Pitch */
            GEN_MODLFOTOPITCH,    /* Modulation LFO to pitch */
            GEN_VIBLFOTOPITCH,    /* Vibrato LFO to pitch */
            GEN_MODENVTOPITCH,    /* Modulation envelope to pitch */
            GEN_MODLFOTOVOL,      /* Modulation LFO to volume */
            GEN_FILTERFC,         /* Filter cutoff */
            GEN_FILTERQ,          /* Filter Q */
            GEN_MODLFOTOFILTERFC, /* Modulation LFO to filter cutoff */
            GEN_MODENVTOFILTERFC, /* Modulation envelope to filter cutoff */
            GEN_CHORUSSEND,       /* Chorus send amount */
            GEN_REVERBSEND,       /* Reverb send amount */
        ];

        let sf2_gen = AWE32_TO_SF2_GEN[gen as usize];
        let mut is_realtime = false;
        let mut converted_sf2_generator_value: FluidReal;

        // The AWE32 NRPN docs say that a value of 8192 is considered to be the middle, i.e. zero.
        // However, it looks like for those generators which work in range [0,127], the AWE32 only inspects the DATA_LSB, i.e. and not doing this subtraction. Found while investigating Uplift.mid.
        data -= 8192;

        match sf2_gen {
            GEN_MODLFODELAY | GEN_VIBLFODELAY | GEN_MODENVDELAY | GEN_VOLENVDELAY => {
                data = fluid_clip(data, 0, 5900);
                converted_sf2_generator_value =
                    fluid_sec2tc(data as FluidReal * (4.0 / 1000.0) as FluidReal);
            }

            GEN_MODLFOFREQ | GEN_VIBLFOFREQ => {
                data_lsb = fluid_clip(data_lsb, 0, 127);
                converted_sf2_generator_value =
                    fluid_hz2ct(data_lsb as FluidReal * 0.084 as FluidReal /* Hz */);
                is_realtime = true;
            }

            GEN_MODENVATTACK | GEN_VOLENVATTACK => {
                data = fluid_clip(data, 0, 5940);
                converted_sf2_generator_value =
                    fluid_sec2tc(data as FluidReal * (1.0 / 1000.0) as FluidReal);
            }

            GEN_MODENVHOLD | GEN_VOLENVHOLD => {
                data = fluid_clip(data, 0, 8191);
                converted_sf2_generator_value =
                    fluid_sec2tc(data as FluidReal * (1.0 / 1000.0) as FluidReal);
            }

            GEN_MODENVDECAY | GEN_MODENVRELEASE | GEN_VOLENVDECAY | GEN_VOLENVRELEASE => {
                data = fluid_clip(data, 0, 5940);
                converted_sf2_generator_value =
                    fluid_sec2tc(data as FluidReal * (4.0 / 1000.0) as FluidReal);
            }

            GEN_MODENVSUSTAIN | GEN_VOLENVSUSTAIN => {
                data_lsb = fluid_clip(data_lsb, 0, 127);
                converted_sf2_generator_value = data_lsb as FluidReal * (0.75 /* dB */ * 10.0) as FluidReal /* cB */;
            }

            GEN_PITCH => {
                converted_sf2_generator_value = (data + 8192) as FluidReal;
                // This has the side effect of manipulating the modulation state of the channel's pitchwheel, but
                // I'll buy it, since pitch bend is not a regular SF2 generator and we do a bit of magic there to
                // make it work
                self.fluid_synth_pitch_bend(chan, converted_sf2_generator_value as i32);
                return;
            }

            GEN_MODLFOTOPITCH | GEN_VIBLFOTOPITCH | GEN_MODENVTOPITCH => {
                if sf2_gen != GEN_MODENVTOPITCH {
                    is_realtime = true;
                }
                /* fallthrough */
                data = fluid_clip(data, -127, 127);
                converted_sf2_generator_value = data as FluidReal * 9.375 as FluidReal /* cents */;
            }

            GEN_MODLFOTOVOL => {
                data_lsb = fluid_clip(data_lsb, 0, 127);
                converted_sf2_generator_value = data_lsb as FluidReal * (0.1875 /* dB */ * 10.0) as FluidReal /* cB */;
                is_realtime = true;
            }

            GEN_FILTERFC => {
                data_lsb = fluid_clip(data_lsb, 0, 127);
                // Yes, DO NOT use data here, Uplift.mid doesn't set DATA_MSB=64, therefore we would always get a negative value after subtracting 8192.
                // Since Uplift.mid sounds fine on hardware though, it seems like AWE32 only inspects DATA_LSB in this case.
                // conversion continues below!
                converted_sf2_generator_value = (data_lsb * 59) as FluidReal /* cents! not as stated Hz by the AWE32 docs!!! */;
                converted_sf2_generator_value += 4500.0 /* cents */; // minimum should be around 110 Hz, derived by experiments, see #1473
                flog!(
                    FLUID_DBG,
                    "AWE32 IIR Fc: {:.6} cents",
                    converted_sf2_generator_value
                );
                is_realtime = true;
            }

            GEN_FILTERQ => {
                data_lsb = fluid_clip(data_lsb, 0, 127);
                converted_sf2_generator_value =
                    data_lsb as FluidReal * ((22.0 /* dB */ / 127.0 /* NPRN steps*/) * 10.0) as FluidReal /* cB */;
                flog!(
                    FLUID_DBG,
                    "AWE32 IIR Q: {:.6} cB",
                    converted_sf2_generator_value
                );
                is_realtime = true;
            }

            // Note: The description in the official "SB AWE32 Developer's Information Pack" is probably wrong.
            // There it says: "Positive data value causes a positive phase (from 0 to maximum) filter modulation
            //  [...] negative data value causes a negative phase [...]"
            // That doesn't make sense. A filter is a causual system - you cannot change its phase independently
            // of the rest. The text a few section above has it correct, there they speak of the filter's
            // cutoff frequency. In that sense, this is probably intended to behave similar to the logic in SF2.
            // PS: Same applies to the GEN_MODENVTOFILTERFC below!
            GEN_MODLFOTOFILTERFC => {
                data = fluid_clip(data, -64, 63);
                converted_sf2_generator_value = data as FluidReal * 56.25 as FluidReal /* cents */;
                flog!(
                    FLUID_DBG,
                    "AWE32 MOD LFO TO FILTER Fc: {:.6} cents",
                    converted_sf2_generator_value
                );
                is_realtime = true;
            }

            GEN_MODENVTOFILTERFC => {
                data = fluid_clip(data, -127, 127);
                converted_sf2_generator_value = data as FluidReal * 56.25 as FluidReal /* cents */;
                flog!(
                    FLUID_DBG,
                    "AWE32 MOD ENV TO FILTER Fc: {:.6} cents",
                    converted_sf2_generator_value
                );
            }

            GEN_REVERBSEND => {
                data = fluid_clip(data, 0, 255);
                /* transform the input value */
                converted_sf2_generator_value = fluid_mod_transform_source_value(
                    &DEFAULT_REVERB_MOD,
                    data as FluidReal,
                    256.0,
                    true,
                );
                flog!(
                    FLUID_DBG,
                    "AWE32 Reverb: {:.6}",
                    converted_sf2_generator_value
                );
                converted_sf2_generator_value *= DEFAULT_REVERB_MOD.amount;
            }

            GEN_CHORUSSEND => {
                data = fluid_clip(data, 0, 255);
                /* transform the input value */
                converted_sf2_generator_value = fluid_mod_transform_source_value(
                    &DEFAULT_CHORUS_MOD,
                    data as FluidReal,
                    256.0,
                    true,
                );
                flog!(
                    FLUID_DBG,
                    "AWE32 Chorus: {:.6}",
                    converted_sf2_generator_value
                );
                converted_sf2_generator_value *= DEFAULT_CHORUS_MOD.amount;
            }

            _ => {
                // should not happen
                flog!(FLUID_WARN, "AWE32 NPRN {} conversion not implemented", gen);
                return;
            }
        }

        self.channel[chan as usize]
            .set_override_gen_default(sf2_gen, converted_sf2_generator_value);

        let mut i = 0;
        while is_realtime && i < self.polyphony as usize {
            let voice = &self.voice[i];

            if fluid_voice_is_playing(voice) && fluid_voice_get_channel(voice) == chan {
                // sets the adjusted generator
                let (voice, mut env) = self.split_voice(i);
                fluid_voice_gen_set(voice, env.eh, sf2_gen, converted_sf2_generator_value as f32);
                fluid_voice_update_param(voice, &mut env, sf2_gen);

                flog!(
                    FLUID_DBG,
                    "AWE32 Realtime: adjusting voice id {}, generator {}, chan {}",
                    voice.id as i32,
                    sf2_gen,
                    chan
                );
            }
            i += 1;
        }
    }

    /// Translation of `fluid_synth_get_active_voice_count()`: get current
    /// number of active voices.
    pub(crate) fn fluid_synth_get_active_voice_count(&mut self) -> i32 {
        self.fluid_synth_api_enter();
        let result = self.active_voice_count;
        self.api_return(result)
    }

    /// Translation of `fluid_synth_get_voicelist()`: the indices of the
    /// playing voices (at most `bufsize`), of all voices if `id` is
    /// negative, otherwise only the ones with that ID.
    pub(crate) fn fluid_synth_get_voicelist(&mut self, bufsize: usize, id: i32) -> Vec<usize> {
        let mut buf = Vec::new();

        self.fluid_synth_api_enter();

        let mut i = 0;
        while i < self.polyphony as usize && buf.len() < bufsize {
            let voice = &self.voice[i];

            if fluid_voice_is_playing(voice) && (id < 0 || voice.id as i32 == id) {
                buf.push(i);
            }
            i += 1;
        }

        self.fluid_synth_api_exit();
        buf
    }

    /// Translation of `fluid_synth_handle_midi_event()`: handle MIDI event
    /// from MIDI router, used as a callback function.
    pub(crate) fn fluid_synth_handle_midi_event(&mut self, event: &FluidMidiEvent) -> i32 {
        let type_ = event.get_type();
        let chan = event.get_channel();

        match type_ {
            NOTE_ON => self.fluid_synth_noteon(chan, event.get_key(), event.get_velocity()),

            NOTE_OFF => self.fluid_synth_noteoff(chan, event.get_key()),

            CONTROL_CHANGE => self.fluid_synth_cc(chan, event.get_control(), event.get_value()),

            PROGRAM_CHANGE => self.fluid_synth_program_change(chan, event.get_program()),

            CHANNEL_PRESSURE => self.fluid_synth_channel_pressure(chan, event.get_program()),

            KEY_PRESSURE => self.fluid_synth_key_pressure(chan, event.get_key(), event.get_value()),

            PITCH_BEND => self.fluid_synth_pitch_bend(chan, event.get_pitch()),

            MIDI_SYSTEM_RESET => self.fluid_synth_system_reset(),

            MIDI_SYSEX => {
                let data = event.sysex_data();
                if data.is_empty() {
                    return FLUID_FAILED;
                }
                self.fluid_synth_sysex(data, None, false)
            }

            MIDI_TEXT | MIDI_LYRIC | MIDI_SET_TEMPO => FLUID_OK,

            _ => FLUID_FAILED,
        }
    }

    /// Translation of `fluid_synth_set_important_channels()`: set the
    /// important channels for voice overflow priority calculation.
    ///
    /// @param channels comma-separated list of channel numbers
    fn fluid_synth_set_important_channels(&mut self, channels: Option<&str>) -> i32 {
        let scores = &mut self.overflow;

        if scores.num_important_channels < self.midi_channels {
            scores
                .important_channels
                .resize(self.midi_channels as usize, 0);

            scores.num_important_channels = self.midi_channels;
        }

        for c in scores.important_channels.iter_mut() {
            *c = 0;
        }

        if let Some(channels) = channels {
            let mut values = vec![0i32; self.midi_channels as usize];

            /* Every channel given in the comma-separated list of channel numbers
             * is set to TRUE, i.e. flagging it as "important". Channel numbers are
             * 1-based. */
            let num_values =
                crate::utils::settings::fluid_settings_split_csv(channels, &mut values);

            for &value in &values[..num_values.max(0) as usize] {
                if value > 0 && value <= self.midi_channels {
                    scores.important_channels[(value - 1) as usize] = 1;
                }
            }
        }

        FLUID_OK
    }

    /// Translation of `fluid_synth_reset_basic_channel_LOCAL()`: resets a
    /// basic channel group of MIDI channels.
    /// @param chan the beginning channel of the group.
    /// @param nbr_chan the number of channel in the group.
    fn fluid_synth_reset_basic_channel_local(&mut self, chan: i32, nbr_chan: i32) {
        for i in chan..chan + nbr_chan {
            let channel = &mut self.channel[i as usize];
            channel.reset_basic_channel_info();
            channel.mode_val = 0;
        }
    }

    /// Translation of `fluid_synth_check_next_basic_channel()`: checks if a
    /// new basic channel group overlaps the next basic channel group.
    ///
    /// On success the function returns the possible number of channel for this
    /// new basic channel group.
    /// The function fails if the new group overlaps the next basic channel group.
    fn fluid_synth_check_next_basic_channel(&self, basicchan: i32, mode: i32, val: i32) -> i32 {
        let n_chan = self.midi_channels; /* MIDI Channels count */
        let mut real_val = val; /* real number of channels in the group */

        /* adjusts val range */
        if mode == FLUID_CHANNEL_MODE_OMNIOFF_POLY {
            real_val = 1; /* mode poly omnioff implies a group of only one channel.*/
        } else if val == 0 {
            /* mode poly omnion (0), mono omnion (1), mono omni off (3) */
            /* value 0 means all possible channels from basicchan to MIDI channel count -1.*/
            real_val = n_chan - basicchan;
        }
        /* checks if val range is above MIDI channel count */
        else if basicchan + val > n_chan {
            return FLUID_FAILED;
        }

        /* checks if this basic channel group overlaps next basic channel group */
        for i in basicchan + 1..basicchan + real_val {
            if self.channel[i as usize].mode & FLUID_CHANNEL_BASIC != 0 {
                /* A value of 0 for val means all possible channels from basicchan to
                to the next basic channel -1 (if any).
                When i reaches the next basic channel group, real_val will be
                limited if it is possible */
                if val == 0 {
                    /* limitation of real_val */
                    real_val = i - basicchan;
                    break;
                }

                /* overlap with the next basic channel group */
                return FLUID_FAILED;
            }
        }

        real_val
    }

    /// Translation of `fluid_synth_set_basic_channel()`: sets a new basic
    /// channel group only. The function doesn't allow to change an
    /// existing basic channel.
    ///
    /// The function fails if any channel overlaps any existing basic channel group.
    /// To make room if necessary, basic channel groups can be cleared using
    /// fluid_synth_reset_basic_channel().
    ///
    /// @param chan the basic Channel number (0 to MIDI channel count-1).
    /// @param mode the MIDI mode to use for chan (see #fluid_basic_channel_modes).
    /// @param val number of channels in the group.
    pub(crate) fn fluid_synth_set_basic_channel(
        &mut self,
        chan: i32,
        mode: i32,
        mut val: i32,
    ) -> i32 {
        /* check parameters */
        if !(0..FLUID_CHANNEL_MODE_LAST).contains(&mode) || val < 0 {
            return FLUID_FAILED;
        }
        if !self.api_entry_chan(chan) {
            return FLUID_FAILED;
        }

        /**/
        if val > 0 && chan + val > self.midi_channels {
            return self.api_return(FLUID_FAILED);
        }

        /* Checks if there is an overlap with the next basic channel */
        val = self.fluid_synth_check_next_basic_channel(chan, mode, val);

        if val == FLUID_FAILED || self.channel[chan as usize].mode & FLUID_CHANNEL_ENABLED != 0 {
            /* overlap with the next or previous channel group */
            flog!(FLUID_INFO, "basic channel {} overlaps another group", chan);
            return self.api_return(FLUID_FAILED);
        }

        /* sets a new basic channel group */
        self.fluid_synth_set_basic_channel_local(chan, mode, val);
        /**/
        self.api_return(FLUID_OK)
    }

    /// Translation of `fluid_synth_set_basic_channel_LOCAL()`: local version
    /// of fluid_synth_set_basic_channel(), called internally:
    /// - by fluid_synth_set_basic_channel() to set a new basic channel group.
    /// - during creation new_fluid_synth() or on CC reset to set a default basic channel group.
    /// - on CC ominoff, CC omnion, CC poly , CC mono to change an existing basic channel group.
    fn fluid_synth_set_basic_channel_local(&mut self, basicchan: i32, mode: i32, val: i32) {
        /* sets the basic channel group */
        for i in basicchan..basicchan + val {
            let mut new_mode = mode; /* OMNI_OFF/ON, MONO/POLY ,others bits are zero */
            let new_val;
            /* MIDI specs: when mode is changed, channel must receive ALL_NOTES_OFF */
            self.fluid_synth_all_notes_off_local(i);

            if i == basicchan {
                new_mode |= FLUID_CHANNEL_BASIC; /* First channel in the group */
                new_val = val; /* number of channels in the group */
            } else {
                new_val = 0; /* val is 0 for other channel than basic channel */
            }

            /* Channel is enabled */
            new_mode |= FLUID_CHANNEL_ENABLED;
            /* Now new_mode is OMNI OFF/ON,MONO/POLY, BASIC_CHANNEL or not and enabled */
            self.channel[i as usize].set_basic_channel_info(new_mode);
            self.channel[i as usize].mode_val = new_val;
        }
    }
}

/// Translation of `fluid_parse_portamento_time_str()`.
fn fluid_parse_portamento_time_str(value: &str) -> i32 {
    match value {
        "auto" => FLUID_PORTAMENTO_TIME_MODE_AUTO,
        "xg-gs" => FLUID_PORTAMENTO_TIME_MODE_XG_GS,
        "linear" => FLUID_PORTAMENTO_TIME_MODE_LINEAR,
        _ => {
            flog!(
                FLUID_ERR,
                "Invalid portamento time mode '{}'. Valid modes: auto, xg-gs, linear",
                value
            );
            FLUID_FAILED
        }
    }
}

impl Synth {
    /// Translation of `new_fluid_synth()`: create new FluidSynth instance.
    ///
    /// The settings are read when the synth is created; changing them
    /// afterwards does not affect it.
    pub fn new(settings: &Settings) -> Option<Synth> {
        let synth = new_fluid_synth(settings)?;
        Some(Synth {
            inner: Arc::new(Mutex::new(synth)),
        })
    }

    /// The locked synth.
    pub(crate) fn lock(&self) -> MutexGuard<'_, FluidSynthInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Translation of `fluid_synth_noteon()`: send a note-on event.
    pub fn noteon(&self, chan: i32, key: i32, vel: i32) -> i32 {
        self.lock().fluid_synth_noteon(chan, key, vel)
    }

    /// Translation of `fluid_synth_noteoff()`: send a note-off event.
    pub fn noteoff(&self, chan: i32, key: i32) -> i32 {
        self.lock().fluid_synth_noteoff(chan, key)
    }

    /// Translation of `fluid_synth_cc()`: send a MIDI controller event on a
    /// MIDI channel.
    pub fn cc(&self, chan: i32, num: i32, val: i32) -> i32 {
        self.lock().fluid_synth_cc(chan, num, val)
    }

    /// Translation of `fluid_synth_program_change()`: send a program change
    /// event on a MIDI channel.
    pub fn program_change(&self, chan: i32, prognum: i32) -> i32 {
        self.lock().fluid_synth_program_change(chan, prognum)
    }

    /// Translation of `fluid_synth_channel_pressure()`.
    pub fn channel_pressure(&self, chan: i32, val: i32) -> i32 {
        self.lock().fluid_synth_channel_pressure(chan, val)
    }

    /// Translation of `fluid_synth_key_pressure()`.
    pub fn key_pressure(&self, chan: i32, key: i32, val: i32) -> i32 {
        self.lock().fluid_synth_key_pressure(chan, key, val)
    }

    /// Translation of `fluid_synth_pitch_bend()`.
    pub fn pitch_bend(&self, chan: i32, val: i32) -> i32 {
        self.lock().fluid_synth_pitch_bend(chan, val)
    }

    /// Translation of `fluid_synth_sysex()` (without a response buffer, see
    /// there): process a MIDI SYSEX message (the bytes between 0xF0 and
    /// 0xF7). Returns the result, and if the message was recognized and
    /// handled.
    pub fn sysex(&self, data: &[u8], dryrun: bool) -> (i32, bool) {
        let data: Vec<i8> = data.iter().map(|&b| b as i8).collect();
        let mut handled = false;
        let result = self
            .lock()
            .fluid_synth_sysex(&data, Some(&mut handled), dryrun);
        (result, handled)
    }

    /// Translation of `fluid_synth_system_reset()`.
    pub fn system_reset(&self) -> i32 {
        self.lock().fluid_synth_system_reset()
    }

    /// Translation of `fluid_synth_all_notes_off()`.
    pub fn all_notes_off(&self, chan: i32) -> i32 {
        self.lock().fluid_synth_all_notes_off(chan)
    }

    /// Translation of `fluid_synth_all_sounds_off()`.
    pub fn all_sounds_off(&self, chan: i32) -> i32 {
        self.lock().fluid_synth_all_sounds_off(chan)
    }

    /// Translation of `fluid_synth_program_reset()`.
    pub fn program_reset(&self) -> i32 {
        self.lock().fluid_synth_program_reset()
    }

    /// Translation of `fluid_synth_add_sfloader()`: add a SoundFont loader
    /// to the synth (tried before the ones added earlier).
    pub fn add_sfloader(&self, loader: SfLoader) {
        self.lock().fluid_synth_add_sfloader(loader);
    }

    /// Translation of `fluid_synth_sfload()`: load a SoundFont file,
    /// returning its ID or #FLUID_FAILED.
    pub fn sfload(&self, filename: &str, reset_presets: bool) -> i32 {
        self.lock().fluid_synth_sfload(filename, reset_presets)
    }

    /// Translation of `fluid_synth_write_float()`: synthesize `len` frames
    /// of float audio to `lout` and `rout` (or both to `lout` if `rout` is
    /// None) at the given offsets and increments. Fails if the buffers are
    /// too short.
    #[allow(clippy::too_many_arguments)]
    pub fn write_float(
        &self,
        len: i32,
        lout: &mut [f32],
        loff: i32,
        lincr: i32,
        rout: Option<&mut [f32]>,
        roff: i32,
        rincr: i32,
    ) -> i32 {
        self.lock()
            .fluid_synth_write_float(len, lout, loff, lincr, rout, roff, rincr)
    }

    /// Translation of `fluid_synth_get_active_voice_count()`.
    pub fn get_active_voice_count(&self) -> i32 {
        self.lock().fluid_synth_get_active_voice_count()
    }

    /// Translation of `fluid_synth_handle_midi_event()`.
    pub fn handle_midi_event(&self, event: &FluidMidiEvent) -> i32 {
        self.lock().fluid_synth_handle_midi_event(event)
    }
}
