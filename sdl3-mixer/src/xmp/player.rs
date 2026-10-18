// Rust translation of src/player.c and src/player.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The player: channels, envelopes, sequencing, and the public
//! `xmp_start_player()`, `xmp_play_frame()`, `xmp_play_buffer()`,
//! `xmp_get_frame_info()` and friends.

/*
 * Sat, 18 Apr 1998 20:23:07 +0200  Frederic Bujon <lvdl@bigfoot.com>
 * Pan effect bug fixed: In Fastracker II the track panning effect erases
 * the instrument panning effect, and the same should happen in xmp.
 */

/*
 * Fri, 26 Jun 1998 13:29:25 -0400 (EDT)
 * Reported by Jared Spiegel <spieg@phair.csh.rit.edu>
 * when the volume envelope is not enabled (disabled) on a sample, and a
 * notoff is delivered to ft2 (via either a noteoff in the note column or
 * command Kxx [where xx is # of ticks into row to give a noteoff to the
 * sample]), ft2 will set the volume of playback of the sample to 00h.
 *
 * Claudio's fix: implementing effect K
 */

use super::common::*;
use super::effects::*;
use super::extras::*;
use super::filter::libxmp_filter_setup;
use super::lfo::*;
use super::med_extras::libxmp_med_hold_retrigger;
use super::mixer::*;
use super::period::*;
use super::read_event::libxmp_read_event;
use super::scan::libxmp_get_sequence;
use super::smix::{libxmp_get_instrument, libxmp_get_sample};
use super::virt::*;
use super::*;

/// Translation of `struct retrig_control`.
pub(crate) struct RetrigControl {
    pub s: i32,
    pub m: i32,
    pub d: i32,
}

/* The following macros are used to set the flags for each channel */
pub(crate) const VOL_SLIDE: i32 = 1 << 0;
pub(crate) const PAN_SLIDE: i32 = 1 << 1;
pub(crate) const TONEPORTA: i32 = 1 << 2;
pub(crate) const PITCHBEND: i32 = 1 << 3;
pub(crate) const VIBRATO: i32 = 1 << 4;
pub(crate) const TREMOLO: i32 = 1 << 5;
pub(crate) const FINE_VOLS: i32 = 1 << 6;
pub(crate) const FINE_BEND: i32 = 1 << 7;
pub(crate) const OFFSET: i32 = 1 << 8;
pub(crate) const TRK_VSLIDE: i32 = 1 << 9;
pub(crate) const TRK_FVSLIDE: i32 = 1 << 10;
pub(crate) const NEW_INS: i32 = 1 << 11;
pub(crate) const NEW_VOL: i32 = 1 << 12;
pub(crate) const VOL_SLIDE_2: i32 = 1 << 13;
pub(crate) const NOTE_SLIDE: i32 = 1 << 14;
pub(crate) const FINE_NSLIDE: i32 = 1 << 15;
pub(crate) const NEW_NOTE: i32 = 1 << 16;
pub(crate) const FINE_TPORTA: i32 = 1 << 17;
pub(crate) const RETRIG: i32 = 1 << 18;
pub(crate) const PANBRELLO: i32 = 1 << 19;
pub(crate) const GVOL_SLIDE: i32 = 1 << 20;
pub(crate) const TEMPO_SLIDE: i32 = 1 << 21;
pub(crate) const VENV_PAUSE: i32 = 1 << 22;
pub(crate) const PENV_PAUSE: i32 = 1 << 23;
pub(crate) const FENV_PAUSE: i32 = 1 << 24;
pub(crate) const FINE_VOLS_2: i32 = 1 << 25;
pub(crate) const KEY_OFF: i32 = 1 << 26; /* for IT release on envloop end */
pub(crate) const TREMOR: i32 = 1 << 27; /* for XM tremor */
pub(crate) const MIDI_MACRO: i32 = 1 << 28; /* IT midi macro */

pub(crate) const NOTE_FADEOUT: i32 = 1 << 0;
pub(crate) const NOTE_ENV_RELEASE: i32 = 1 << 1; /* envelope sustain loop release */
pub(crate) const NOTE_END: i32 = 1 << 2;
pub(crate) const NOTE_CUT: i32 = 1 << 3;
pub(crate) const NOTE_ENV_END: i32 = 1 << 4;
pub(crate) const NOTE_SAMPLE_END: i32 = 1 << 5;
pub(crate) const NOTE_SET: i32 = 1 << 6; /* for IT portamento after keyoff */
pub(crate) const NOTE_SUSEXIT: i32 = 1 << 7; /* for delayed envelope release */
pub(crate) const NOTE_KEY_CUT: i32 = 1 << 8; /* note cut with XMP_KEY_CUT event */
pub(crate) const NOTE_GLISSANDO: i32 = 1 << 9;
pub(crate) const NOTE_SAMPLE_RELEASE: i32 = 1 << 10; /* sample sustain loop release */

/* Most of the time, these should be set/reset together. */
pub(crate) const NOTE_RELEASE: i32 = NOTE_ENV_RELEASE | NOTE_SAMPLE_RELEASE;

/* Note: checking the data pointer for samples should be good enough to filter
 * broken samples, since libxmp_load_sample will always allocate it for valid
 * samples of >0 length and bound the loop values for these samples. */
/// `IS_VALID_INSTRUMENT(x)`
#[inline]
pub(crate) fn is_valid_instrument(mod_: &XmpModule, x: i32) -> bool {
    (x as u32) < mod_.ins as u32 && mod_.xxi[x as usize].nsm > 0
}
/// `IS_VALID_INSTRUMENT_OR_SFX(x)`
#[inline]
pub(crate) fn is_valid_instrument_or_sfx(mod_: &XmpModule, smix: &SmixData, x: i32) -> bool {
    ((x as u32) < mod_.ins as u32 && mod_.xxi[x as usize].nsm > 0)
        || (smix.ins > 0 && (x as u32) < (mod_.ins + smix.ins) as u32)
}
/// `IS_VALID_SAMPLE(x)`
#[inline]
pub(crate) fn is_valid_sample(mod_: &XmpModule, x: i32) -> bool {
    (x as u32) < mod_.smp as u32 && mod_.xxs[x as usize].data.is_some()
}
/// `IS_VALID_NOTE(x)`
#[inline]
pub(crate) fn is_valid_note(x: i32) -> bool {
    (x as u32) < XMP_MAX_KEYS as u32
}

/// Translation of `struct instrument_vibrato`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InstrumentVibrato {
    pub phase: i32,
    pub sweep: i32,
}

/// An LFO effect's state and memory (the `vibrato`, `tremolo` and
/// `panbrello` of `struct channel_data`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LfoMem {
    pub lfo: Lfo,
    pub memory: i32,
}

/// The `arpeggio` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Arpeggio {
    pub val: [i8; 16], /* 16 for Smaksak MegaArps */
    pub size: i32,
    pub count: i32,
    pub memory: i32,
}

/// The `insvib` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InsVib {
    pub lfo: Lfo,
    pub sweep: i32,
}

/// The `offset` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Offset {
    pub val: i32,
    pub val2: i32, /* For fx9 bug emulation */
    pub memory: i32,
}

/// The `retrig` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Retrig {
    pub val: i32,   /* Retrig value */
    pub count: i32, /* Retrig counter */
    pub type_: i32, /* Retrig type */
    pub limit: i32, /* Number of retrigs */
}

pub(crate) const TREMOR_SUPPRESS: u8 = 0x40; /* Ignore tremor state until next update (FT2) */
pub(crate) const TREMOR_ON: u8 = 0x80;

/// The `tremor` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Tremor {
    pub up: u8,
    pub down: u8,   /* Tremor value */
    pub count: u8,  /* Tremor counter */
    pub memory: u8, /* Tremor memory */
}

/// The `vol` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Vol {
    pub slide: i32,  /* Volume slide value */
    pub fslide: i32, /* Fine volume slide value */
    pub slide2: i32, /* Volume slide value */
    pub memory: i32, /* Volume slide effect memory */
    pub fslide2: i32,
    pub memory2: i32, /* Volume slide effect memory */
    pub target: i32,  /* Target for persistent volslide */
}

/// The `fine_vol` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FineVol {
    pub up_memory: i32,   /* Fine volume slide up memory (XM) */
    pub down_memory: i32, /* Fine volume slide up memory (XM) */
}

/// The `gvol` and `trackvol` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Slide {
    pub slide: i32,  /* Global volume slide value */
    pub fslide: i32, /* Fine global volume slide value */
    pub memory: i32, /* Global volume memory is saved per channel */
}

/// The `freq` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Freq {
    pub slide: i32,       /* Frequency slide value */
    pub fslide: f64,      /* Fine frequency slide value */
    pub memory: i32,      /* Portamento effect memory */
    pub down_memory: i32, /* Portamento down effect memory (XM) */
}

/// The `porta` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Porta {
    pub target: f64,      /* Target period for tone portamento */
    pub dir: i32,         /* Tone portamento up/down direction */
    pub slide: i32,       /* Delta for tone portamento */
    pub memory: i32,      /* Tone portamento effect memory */
    pub note_memory: i32, /* Tone portamento note memory (ULT) */
}

/// The `fine_porta` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FinePorta {
    pub up_memory: i32,   /* FT2 has separate memories for these */
    pub down_memory: i32, /* cases (see Porta-LinkMem.xm) */
    pub xf_up_memory: i32,
    pub xf_down_memory: i32,
}

/// The `pan` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pan {
    pub val: i32,      /* Current pan value */
    pub slide: i32,    /* Pan slide value */
    pub fslide: i32,   /* Pan fine slide value */
    pub memory: i32,   /* Pan slide effect memory */
    pub surround: i32, /* Surround channel flag */
}

/// The `invloop` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InvLoop {
    pub speed: i32,
    pub count: i32,
    pub pos: i32,
}

/// The `filter` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ChnFilter {
    pub cutoff: i32,      /* IT filter cutoff frequency */
    pub resonance: i32,   /* IT filter resonance */
    pub envelope: i32,    /* IT filter envelope */
    pub can_disable: i32, /* IT hack: allow disabling for cutoff 127 */
}

/// The `macro` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Macro {
    pub val: f32,      /* Current macro effect (use float for slides) */
    pub target: f32,   /* Current macro target (smooth macro) */
    pub slide: f32,    /* Current macro slide (smooth macro) */
    pub active: i32,   /* Current active parameterized macro */
    pub finalvol: i32, /* Previous tick calculated volume (0-0x400) */
    pub notepan: i32,  /* Previous tick note panning (0x80 center) */
}

/// The `noteslide` of `struct channel_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NoteSlide {
    pub slide: i32,  /* PTM note slide amount */
    pub fslide: i32, /* OKT fine note slide amount */
    pub speed: i32,  /* PTM note slide speed */
    pub count: i32,  /* PTM note slide counter */
}

/// Translation of `struct channel_data`.
#[derive(Clone, Debug, Default)]
pub(crate) struct ChannelData {
    pub flags: i32,      /* Channel flags */
    pub per_flags: i32,  /* Persistent effect channel flags */
    pub note_flags: i32, /* Note release, fadeout or end */
    pub note: i32,       /* Note number */
    pub key: i32,        /* Key number */
    pub period: f64,     /* Amiga or linear period */
    pub per_adj: f64,    /* MED period/pitch adjustment factor hack */
    pub finetune: i32,   /* Guess what */
    pub ins: i32,        /* Instrument number */
    pub old_ins: i32,    /* Last instrument */
    pub smp: i32,        /* Sample number */
    pub mastervol: i32,  /* Master vol -- for IT track vol effect */
    pub delay: i32,      /* Note delay in frames */
    pub keyoff: i32,     /* Key off counter */
    pub fadeout: i32,    /* Current fadeout (release) value */
    pub ins_fade: i32,   /* Instrument fadeout value */
    pub volume: i32,     /* Current volume */
    pub gvl: i32,        /* Global volume for instrument for IT */

    pub rvv: i32, /* Random volume variation */
    pub rpv: i32, /* Random pan variation */

    pub split: u8, /* Split channel */
    pub pair: u8,  /* Split channel pair */

    pub v_idx: i32, /* Volume envelope index */
    pub p_idx: i32, /* Pan envelope index */
    pub f_idx: i32, /* Freq envelope index */

    pub key_porta: i32, /* Key number for portamento target
                         * -- needed to handle IT portamento xpo */
    pub vibrato: LfoMem,

    pub tremolo: LfoMem,

    pub panbrello: LfoMem,

    pub arpeggio: Arpeggio,

    pub insvib: InsVib,

    pub offset: Offset,

    pub retrig: Retrig,

    pub tremor: Tremor,

    pub vol: Vol,

    pub fine_vol: FineVol,

    pub gvol: Slide,

    pub trackvol: Slide,

    pub freq: Freq,

    pub porta: Porta,

    pub fine_porta: FinePorta,

    pub pan: Pan,

    pub invloop: InvLoop,

    pub tempo_slide: i32, /* IT tempo slide (tempo.slide) */

    pub filter: ChnFilter,

    pub macro_: Macro,

    pub noteslide: NoteSlide,

    pub extra: ChannelExtras,

    pub delayed_event: XmpEvent,
    pub delayed_ins: i32, /* IT save instrument emulation */
    pub key_memory: i32,  /* Previous key (XM) */

    pub info_period: i32,    /* Period */
    pub info_pitchbend: i32, /* Linear pitchbend */
    pub info_position: i32,  /* Position before mixing */
    pub info_finalvol: i32,  /* Final volume including envelopes */
    pub info_finalpan: i32,  /* Final pan including envelopes */
}

impl ChannelData {
    /* Channel flag control */
    /// `SET(f)`
    #[inline]
    pub fn set(&mut self, f: i32) {
        self.flags |= f;
    }
    /// `RESET(f)`
    #[inline]
    pub fn reset(&mut self, f: i32) {
        self.flags &= !f;
    }
    /// `TEST(f)`
    #[inline]
    pub fn test(&self, f: i32) -> bool {
        self.flags & f != 0
    }

    /* Persistent effect flag control */
    /// `SET_PER(f)`
    #[inline]
    pub fn set_per(&mut self, f: i32) {
        self.per_flags |= f;
    }
    /// `RESET_PER(f)`
    #[inline]
    pub fn reset_per(&mut self, f: i32) {
        self.per_flags &= !f;
    }
    /// `TEST_PER(f)`
    #[inline]
    pub fn test_per(&self, f: i32) -> bool {
        self.per_flags & f != 0
    }

    /* Note flag control */
    /// `SET_NOTE(f)`
    #[inline]
    pub fn set_note(&mut self, f: i32) {
        self.note_flags |= f;
    }
    /// `RESET_NOTE(f)`
    #[inline]
    pub fn reset_note(&mut self, f: i32) {
        self.note_flags &= !f;
    }
    /// `TEST_NOTE(f)`
    #[inline]
    pub fn test_note(&self, f: i32) -> bool {
        self.note_flags & f != 0
    }
}

/* Values for multi-retrig */
static RVAL: [RetrigControl; 17] = [
    RetrigControl { s: 0, m: 1, d: 1 },
    RetrigControl { s: -1, m: 1, d: 1 },
    RetrigControl { s: -2, m: 1, d: 1 },
    RetrigControl { s: -4, m: 1, d: 1 },
    RetrigControl { s: -8, m: 1, d: 1 },
    RetrigControl { s: -16, m: 1, d: 1 },
    RetrigControl { s: 0, m: 2, d: 3 },
    RetrigControl { s: 0, m: 1, d: 2 },
    RetrigControl { s: 0, m: 1, d: 1 },
    RetrigControl { s: 1, m: 1, d: 1 },
    RetrigControl { s: 2, m: 1, d: 1 },
    RetrigControl { s: 4, m: 1, d: 1 },
    RetrigControl { s: 8, m: 1, d: 1 },
    RetrigControl { s: 16, m: 1, d: 1 },
    RetrigControl { s: 0, m: 3, d: 2 },
    RetrigControl { s: 0, m: 2, d: 1 },
    RetrigControl { s: 0, m: 0, d: 1 }, /* Note cut */
];

/*
 * "Anyway I think this is the most brilliant piece of crap we
 *  have managed to put up!"
 *			  -- Ice of FC about "Mental Surgery"
 */

/* Envelope */

/// Translation of `check_envelope_end()`.
fn check_envelope_end(env: &XmpEnvelope, x: i32) -> i32 {
    if !env.flg & XMP_ENVELOPE_ON != 0 || env.npt <= 0 {
        return 0;
    }

    let idx = (env.npt - 1) * 2;

    /* last node */
    if (x >= env.d(idx) || idx == 0) && !env.flg & XMP_ENVELOPE_LOOP != 0 {
        return 1;
    }

    0
}

/// Translation of `get_envelope()`.
fn get_envelope(env: &XmpEnvelope, x: i32, def: i32) -> i32 {
    if x < 0 || !env.flg & XMP_ENVELOPE_ON != 0 || env.npt <= 0 {
        return def;
    }

    let mut idx = (env.npt - 1) * 2;

    let mut x1 = env.d(idx); /* last node */
    if x >= x1 || idx == 0 {
        return env.d(idx + 1);
    }

    loop {
        idx -= 2;
        x1 = env.d(idx);
        if !(idx > 0 && x1 > x) {
            break;
        }
    }

    /* interpolate */
    let y1 = env.d(idx + 1);
    let x2 = env.d(idx + 2);
    let y2 = env.d(idx + 3);

    /* Interpolation requires x1 <= x <= x2 */
    if x < x1 || x2 < x1 {
        return y1;
    }

    if x2 == x1 {
        y2
    } else {
        ((y2 - y1) * (x - x1) / (x2 - x1)) + y1
    }
}

/// Translation of `update_envelope_generic()`.
fn update_envelope_generic(env: &XmpEnvelope, mut x: i32, mut release: i32) -> i32 {
    let has_loop = env.flg & XMP_ENVELOPE_LOOP;
    let mut has_sus = env.flg & XMP_ENVELOPE_SUS;

    let lps = env.lps << 1;
    let lpe = env.lpe << 1;
    let sus = env.sus << 1;

    /* FT2 and IT envelopes behave in a different way regarding loops,
     * sustain and release. When the sustain point is at the end of the
     * envelope loop end and the key is released, FT2 escapes the loop
     * while IT runs another iteration. (See EnvLoops.xm in the OpenMPT
     * test cases.)
     * TODO: this is a bit suspicious, has little relation to the above
     * description, and had to be removed from the XM handler because it
     * broke a module (fade_2_grey_visage.xm). Retesting is required.
     */
    if has_loop != 0 && has_sus != 0 && sus == lpe && release == 0 {
        has_sus = 0;
    }

    /* If the envelope point is set to somewhere after the sustain point
     * or sustain loop, enable release to prevent the envelope point from
     * returning to the sustain point or loop start. (See Filip Skutela's
     * farewell_tear.xm.)
     */
    if has_loop != 0 && x > env.d(lpe) + 1 {
        release = 1;
    } else if has_sus != 0 && x > env.d(sus) + 1 {
        release = 1;
    }

    /* If enabled, stay at the sustain point */
    if has_sus != 0 && release == 0 && x >= env.d(sus) {
        x = env.d(sus);
    }

    /* XM-like formats and players assume that an envelope position past the
     * end of the loop or sustain point should return to the loop/sustain point.
     * While there are some differences with sustain points, this general loop
     * behavior is used by DigiBooster Pro, Digitrakker, Imago Orpheus, and
     * Real Tracker 2.
     */
    if has_loop != 0 && x >= env.d(lpe) {
        /* FT2 and IT envelopes behave in a different way regarding
         * loops, sustain and release. When the sustain point is at the
         * end of the envelope loop end and the key is released, FT2
         * escapes the loop while IT runs another iteration.
         * (See OpenMPT EnvLoops.xm)
         */
        if !(release != 0 && has_sus != 0 && sus == lpe) {
            x = env.d(lps);
        }
    }

    x
}

/// Translation of `update_envelope_xm()`.
fn update_envelope_xm(env: &XmpEnvelope, mut x: i32, mut release: i32) -> i32 {
    let has_loop = env.flg & XMP_ENVELOPE_LOOP;
    let has_sus = env.flg & XMP_ENVELOPE_SUS;

    let lps = env.lps << 1;
    let lpe = env.lpe << 1;
    let sus = env.sus << 1;

    /* If the envelope point is set to somewhere after the sustain point
     * or sustain loop, enable release to prevent the envelope point from
     * returning to the sustain point or loop start. (See Filip Skutela's
     * farewell_tear.xm.)
     */
    if has_sus != 0 && x > env.d(sus) + 1 {
        release = 1;
    }

    /* If enabled, stay at the sustain point */
    if has_sus != 0 && release == 0 && x >= env.d(sus) {
        x = env.d(sus);
    }

    /* Envelope loops
     *
     * If the envelope point is set to somewhere after the sustain point
     * or sustain loop, the loop point is ignored to prevent the envelope
     * point from returning to the sustain point or loop start.
     * (See Filip Skutela's farewell_tear.xm or Ebony Owl Netsuke.xm.)
     */
    if has_loop != 0 && x == env.d(lpe) {
        /* FT2 and IT envelopes behave in a different way regarding
         * loops, sustain and release. When the sustain point is at the
         * end of the envelope loop end and the key is released, FT2
         * escapes the loop while IT runs another iteration.
         * (See OpenMPT EnvLoops.xm)
         */
        if !(release != 0 && has_sus != 0 && sus == lpe) {
            x = env.d(lps);
        }
    }

    x
}

/// Translation of `update_envelope_it()`.
fn update_envelope_it(env: &XmpEnvelope, mut x: i32, release: i32, key_off: i32) -> i32 {
    let has_loop = env.flg & XMP_ENVELOPE_LOOP;
    let has_sus = env.flg & XMP_ENVELOPE_SUS;

    let lps = env.lps << 1;
    let lpe = env.lpe << 1;
    let sus = env.sus << 1;
    let sue = env.sue << 1;

    /* Release at the end of a sustain loop, run another loop */
    if has_sus != 0 && key_off != 0 && x == env.d(sue) + 1 {
        x = env.d(sus);
    } else
    /* If enabled, stay in the sustain loop */
    if has_sus != 0 && release == 0 {
        if x == env.d(sue) + 1 {
            x = env.d(sus);
        }
    } else
    /* Finally, execute the envelope loop */
    if has_loop != 0 && x > env.d(lpe) {
        x = env.d(lps);
    }

    x
}

/// Translation of `update_envelope()`.
fn update_envelope(
    m: &ModuleData,
    env: &XmpEnvelope,
    mut x: i32,
    release: i32,
    key_off: i32,
) -> i32 {
    if x < 0xffff {
        /* increment tick */
        x += 1;
    }

    if x < 0 {
        return -1;
    }

    if !env.flg & XMP_ENVELOPE_ON != 0 || env.npt <= 0 {
        return x;
    }

    if m.is_player_mode_it() {
        update_envelope_it(env, x, release, key_off)
    } else if !m.has_quirk(QUIRK_FT2ENV) {
        update_envelope_generic(env, x, release)
    } else {
        update_envelope_xm(env, x, release)
    }
}

/* Returns: 0 if do nothing, <0 to reset channel, >0 if has fade */
/// Translation of `check_envelope_fade()`.
fn check_envelope_fade(env: &XmpEnvelope, x: i32) -> i32 {
    if !env.flg & XMP_ENVELOPE_ON != 0 {
        return 0;
    }

    let idx = (env.npt - 1) * 2; /* last node */
    if x > env.d(idx) {
        if env.d(idx + 1) == 0 {
            return -1;
        } else {
            return 1;
        }
    }

    0
}

/* Impulse Tracker's filter effects are implemented using its MIDI macros.
 * Any module can customize these and they are parameterized using various
 * player and mixer values, which requires parsing them here instead of in
 * the loader. Since they're MIDI macros, they can contain actual MIDI junk
 * that needs to be skipped, and one macro may have multiple IT commands. */

/// Translation of `struct midi_stream`.
struct MidiStream {
    data: [u8; 32],
    pos: usize,
    buffer: i32,
    param: i32,
}

/// Translation of `midi_nibble()`.
fn midi_nibble(ctx: &mut ContextData, chn: i32, in_: &mut MidiStream) -> i32 {
    let mut byte = -1;
    if in_.buffer >= 0 {
        let val = in_.buffer;
        in_.buffer = -1;
        return val;
    }

    while in_.pos < in_.data.len() && in_.data[in_.pos] != 0 {
        let val = in_.data[in_.pos] as i32;
        in_.pos += 1;
        if val >= b'0' as i32 && val <= b'9' as i32 {
            return val - b'0' as i32;
        }
        if val >= b'A' as i32 && val <= b'F' as i32 {
            return val - b'A' as i32 + 10;
        }
        let xc = &ctx.p.xc_data[chn as usize];
        match val as u8 {
            b'z' => {
                /* Macro parameter */
                byte = in_.param;
            }
            b'n' => {
                /* Host key */
                byte = xc.key & 0x7f;
            }
            b'h' => {
                /* Host channel */
                byte = chn;
            }
            b'o' => {
                /* Offset effect memory */
                /* Intentionally not clamped, see ZxxSecrets.it */
                byte = xc.offset.memory;
            }
            b'm' => {
                /* Voice reverse flag */
                let voc = libxmp_virt_mapchannel(ctx, chn);
                byte = if voc >= 0 {
                    (ctx.p.virt.voice_array[voc as usize].flags & VOICE_REVERSE != 0) as i32
                } else {
                    0
                };
            }
            b'v' => {
                /* Note velocity */
                let xxi_vol = libxmp_get_instrument(ctx, xc.ins).map_or(0x40, |x| x.vol);
                byte = ((ctx.p.gvol as u32)
                    .wrapping_mul(xc.volume as u32)
                    .wrapping_mul(xc.mastervol as u32)
                    .wrapping_mul(xc.gvl as u32)
                    .wrapping_mul(xxi_vol as u32)
                    >> 24) as i32;
                clamp(&mut byte, 1, 127);
            }
            b'u' => {
                /* Computed velocity */
                byte = xc.macro_.finalvol >> 3;
                clamp(&mut byte, 1, 127);
            }
            b'x' => {
                /* Note panning */
                byte = xc.macro_.notepan >> 1;
                clamp(&mut byte, 0, 127);
            }
            b'y' => {
                /* Computed panning */
                byte = xc.info_finalpan >> 1;
                clamp(&mut byte, 0, 127);
            }
            b'a' | b'b' | b'p' | b's' => {
                /* Ins MIDI Bank hi */
                /* Ins MIDI Bank lo */
                /* Ins MIDI Program */
                /* MPT: SysEx checksum */
                byte = 0;
            }
            b'c' => {
                /* Ins MIDI Channel */
                return 0;
            }
            _ => {}
        }

        /* Byte output */
        if byte >= 0 {
            in_.buffer = byte & 0xf;
            return (byte >> 4) & 0xf;
        }
    }
    -1
}

/// Translation of `midi_byte()`.
fn midi_byte(ctx: &mut ContextData, chn: i32, in_: &mut MidiStream) -> i32 {
    let a = midi_nibble(ctx, chn, in_);
    let b = midi_nibble(ctx, chn, in_);
    if a >= 0 && b >= 0 {
        (a << 4) | b
    } else {
        -1
    }
}

/// Translation of `apply_midi_macro_effect()`.
fn apply_midi_macro_effect(xc: &mut ChannelData, type_: i32, val: i32) {
    match type_ {
        0 => {
            /* Filter cutoff */
            xc.filter.cutoff = val << 1;
        }
        1 => {
            /* Filter resonance */
            xc.filter.resonance = val << 1;
        }
        _ => {}
    }
}

/// Translation of `execute_midi_macro()`.
fn execute_midi_macro(ctx: &mut ContextData, chn: i32, midi: &MidiMacro, param: i32) {
    let mut in_ = MidiStream {
        data: midi.data,
        pos: 0,
        buffer: -1,
        param,
    };

    while in_.pos < in_.data.len() && in_.data[in_.pos] != 0 {
        /* Very simple MIDI 1.0 parser--most bytes can just be ignored
         * (or passed through, if libxmp gets MIDI output). All bytes
         * with bit 7 are statuses which interrupt unfinished messages
         * ("Data Types: Status Bytes") or are real time messages.
         * This holds even for SysEx messages, which end at ANY non-
         * real time status ("System Common Messages: EOX").
         *
         * IT intercepts internal "messages" that begin with F0 F0,
         * which in MIDI is a useless zero-length SysEx followed by
         * a second SysEx. They are four bytes long including F0 F0,
         * and shouldn't be passed through. OpenMPT also uses F0 F1.
         */
        let mut cmd = -1;
        let mut byte = midi_byte(ctx, chn, &mut in_);
        if byte == 0xf0 {
            byte = midi_byte(ctx, chn, &mut in_);
            if byte == 0xf0 || byte == 0xf1 {
                cmd = byte & 0xf;
            }
        }
        if cmd < 0 {
            if byte == 0xfa || byte == 0xfc || byte == 0xff {
                /* These real time statuses can appear anywhere
                 * (even in SysEx) and reset the channel filter
                 * params. See: OpenMPT ZxxSecrets.it */
                let xc = &mut ctx.p.xc_data[chn as usize];
                apply_midi_macro_effect(xc, 0, 127);
                apply_midi_macro_effect(xc, 1, 0);
            }
            continue;
        }
        cmd = midi_byte(ctx, chn, &mut in_) | (cmd << 8);
        let val = midi_byte(ctx, chn, &mut in_);
        if cmd < 0 || cmd >= 0x80 || val < 0 || val >= 0x80 {
            continue;
        }
        apply_midi_macro_effect(&mut ctx.p.xc_data[chn as usize], cmd, val);
    }
}

/* This needs to occur before all process_* functions:
 * - It modifies the filter parameters, used by process_frequency.
 * - process_volume and process_pan apply slide effects, which the
 *   filter parameters expect to occur after macro effect parsing. */
/// Translation of `update_midi_macro()`.
fn update_midi_macro(ctx: &mut ContextData, chn: i32) {
    let frame = ctx.p.frame;
    let xc = &mut ctx.p.xc_data[chn as usize];

    if xc.test(MIDI_MACRO) && ctx.m.has_quirk(QUIRK_FILTER) {
        if xc.macro_.slide > 0.0 {
            xc.macro_.val += xc.macro_.slide;
            if xc.macro_.val > xc.macro_.target {
                xc.macro_.val = xc.macro_.target;
                xc.macro_.slide = 0.0;
            }
        } else if xc.macro_.slide < 0.0 {
            xc.macro_.val += xc.macro_.slide;
            if xc.macro_.val < xc.macro_.target {
                xc.macro_.val = xc.macro_.target;
                xc.macro_.slide = 0.0;
            }
        } else if frame != 0 {
            /* Execute non-smooth macros on frame 0 only */
            return;
        }

        let val = xc.macro_.val as i32;
        let active = xc.macro_.active;
        if val >= 0x80 {
            if let Some(midicfg) = ctx.m.midi.as_ref() {
                let macro_ = midicfg.fixed[(val - 0x80) as usize];
                execute_midi_macro(ctx, chn, &macro_, val);
            } else if val < 0x90 {
                /* Default fixed macro: set resonance */
                apply_midi_macro_effect(xc, 1, (val - 0x80) << 3);
            }
        } else if let Some(midicfg) = ctx.m.midi.as_ref() {
            let macro_ = midicfg.param[active as usize];
            execute_midi_macro(ctx, chn, &macro_, val);
        } else if active == 0 {
            /* Default parameterized macro 0: set filter cutoff */
            apply_midi_macro_effect(xc, 0, val);
        }
    }
}

/* From http://www.un4seen.com/forum/?topic=7554.0
 *
 * "Invert loop" effect replaces (!) sample data bytes within loop with their
 * bitwise complement (NOT). The parameter sets speed of altering the samples.
 * This effectively trashes the sample data. Because of that this effect was
 * supposed to be removed in the very next ProTracker versions, but it was
 * never removed.
 *
 * Prior to [Protracker 1.1A] this effect is called "Funk Repeat" and it moves
 * loop of the instrument (just the loop information - sample data is not
 * altered). The parameter is the speed of moving the loop.
 */

static INVLOOP_TABLE: [i32; 16] = [0, 5, 6, 7, 8, 10, 11, 13, 16, 19, 22, 26, 32, 43, 64, 128];

/// Translation of `update_invloop()`.
fn update_invloop(ctx: &mut ContextData, chn: usize) {
    let frame = ctx.p.frame;
    let smp = ctx.p.xc_data[chn].smp;
    let mut lps = -1;
    let mut lpe = -1;
    let mut len = -1;

    let xxs = libxmp_get_sample(ctx, smp).map(|x| (x.len, x.flg));
    let m = &ctx.m;
    let xc = &mut ctx.p.xc_data[chn];

    /* If an instrument number is present, reset the position. */
    if frame == 0 && xc.test(NEW_INS) {
        xc.invloop.pos = 0;
    }

    xc.invloop.count += INVLOOP_TABLE[xc.invloop.speed as usize & 15];

    if let Some((xlen, xflg)) = xxs {
        len = xlen;
        if xflg & XMP_SAMPLE_LOOP != 0 {
            let x = &m.mod_.xxs[smp as usize];
            lps = x.lps;
            lpe = x.lpe;
        } else if xflg & XMP_SAMPLE_SLOOP != 0 {
            /* Some formats that support invert loop use sustain
             * loops instead (Digital Symphony). */
            lps = m.xtra[smp as usize].sus;
            lpe = m.xtra[smp as usize].sue;
        }
    }

    if lps >= 0 && lps <= lpe && xc.invloop.count >= 128 {
        xc.invloop.count = 0;

        xc.invloop.pos += 1;
        if xc.invloop.pos > lpe - lps {
            xc.invloop.pos = 0;
        }

        let pos = xc.invloop.pos;
        let xxs = &mut ctx.m.mod_.xxs[smp as usize];
        if xxs.data.is_none() || lps >= len || lpe > len {
            return;
        }

        if !xxs.flg & XMP_SAMPLE_16BIT != 0 {
            let data = xxs.data.as_mut().unwrap();
            data[SAMPLE_GUARD + (lps + pos) as usize] ^= 0xff;
        }
    }
}

/*
 * From OpenMPT Arpeggio.xm test:
 *
 * "[FT2] Arpeggio behavior is very weird with more than 16 ticks per row. This
 *  comes from the fact that Fasttracker 2 uses a LUT for computing the arpeggio
 *  note (instead of doing something like tick%3 or similar). The LUT only has
 *  16 entries, so when there are more than 16 ticks, it reads beyond array
 *  boundaries. The vibrato table happens to be stored right after arpeggio
 *  table. The tables look like this in memory:
 *
 *    ArpTab: 0,1,2,0,1,2,0,1,2,0,1,2,0,1,2,0
 *    VibTab: 0,24,49,74,97,120,141,161,180,197,...
 *
 *  All values except for the first in the vibrato table are greater than 1, so
 *  they trigger the third arpeggio note. Keep in mind that Fasttracker 2 counts
 *  downwards, so the table has to be read from back to front, i.e. at 16 ticks
 *  per row, the 16th entry in the LUT is the first to be read. This is also the
 *  reason why Arpeggio is played 'backwards' in Fasttracker 2."
 */
/// Translation of `ft2_arpeggio()`.
fn ft2_arpeggio(p: &PlayerData, xc: &ChannelData) -> i32 {
    if xc.arpeggio.val[1] == 0 && xc.arpeggio.val[2] == 0 {
        return 0;
    }

    if p.frame == 0 {
        return 0;
    }

    let i = p.speed - (p.frame % p.speed);

    if i == 16 {
        return 0;
    } else if i > 16 {
        return xc.arpeggio.val[2] as i32;
    }

    xc.arpeggio.val[(i % 3) as usize] as i32
}

/// Translation of `arpeggio()`.
fn arpeggio(ctx: &mut ContextData, chn: usize) -> i32 {
    let p = &ctx.p;
    let xc = &p.xc_data[chn];

    let arp = if ctx.m.has_quirk(QUIRK_FT2BUGS) {
        ft2_arpeggio(p, xc)
    } else {
        xc.arpeggio.val[xc.arpeggio.count as usize] as i32
    };

    let xc = &mut ctx.p.xc_data[chn];
    xc.arpeggio.count += 1;
    xc.arpeggio.count %= xc.arpeggio.size;

    arp
}

/// Translation of `is_first_frame()`.
fn is_first_frame(ctx: &ContextData) -> bool {
    let p = &ctx.p;

    match ctx.m.read_event_type {
        READ_EVENT_IT | READ_EVENT_ST3 => p.frame % p.speed == 0,
        _ => p.frame == 0,
    }
}

/// Translation of `reset_channels()`.
fn reset_channels(ctx: &mut ContextData) {
    /* Default instrument memory: 0 = no ins., 1 = ins. 0, etc.
     * This should be no ins. for FT2 (ft2_instrument_memory_default.xm,
     * "playful girl" by Drozerix) and IT (it_instrument_memory_default.it).
     */
    let old_ins = 0;

    for i in 0..ctx.p.virt.virt_channels as usize {
        let extra = std::mem::take(&mut ctx.p.xc_data[i].extra);
        ctx.p.xc_data[i] = ChannelData::default();
        ctx.p.xc_data[i].extra = extra;
        let mut xc = std::mem::take(&mut ctx.p.xc_data[i]);
        libxmp_reset_channel_extras(ctx, &mut xc);
        xc.ins = -1;
        xc.old_ins = old_ins;
        xc.key = -1;
        xc.volume = ctx.m.volbase;
        ctx.p.xc_data[i] = xc;
    }

    let mod_ = &ctx.m.mod_;
    let smix = &ctx.smix;
    for i in 0..ctx.p.virt.num_tracks as usize {
        {
            let xc = &mut ctx.p.xc_data[i];

            if i as i32 >= mod_.chn && (i as i32) < mod_.chn + smix.chn {
                xc.mastervol = 0x40;
                xc.pan.val = 0x80;
            } else {
                xc.mastervol = mod_.xxc[i].vol;
                xc.pan.val = mod_.xxc[i].pan;
            }

            xc.filter.cutoff = 0xff;
        }

        /* Amiga split channel */
        if mod_.xxc[i].flg & XMP_CHANNEL_SPLIT != 0 {
            let split = (((mod_.xxc[i].flg & 0x30) >> 4) + 1) as u8;
            ctx.p.xc_data[i].split = split;
            /* Connect split channel pairs */
            for j in 0..i {
                if mod_.xxc[j].flg & XMP_CHANNEL_SPLIT != 0 && ctx.p.xc_data[j].split == split {
                    ctx.p.xc_data[j].pair = i as u8;
                    ctx.p.xc_data[i].pair = j as u8;
                }
            }
        } else {
            ctx.p.xc_data[i].split = 0;
        }

        /* Surround channel */
        if mod_.xxc[i].flg & XMP_CHANNEL_SURROUND != 0 {
            ctx.p.xc_data[i].pan.surround = 1;
        }
    }
}

/// Translation of `check_delay()`.
fn check_delay(ctx: &mut ContextData, e: &XmpEvent, chn: usize) -> i32 {
    let p = &mut ctx.p;

    /* Tempo affects delay and must be computed first */
    if ((e.fxt == FX_SPEED && e.fxp < 0x20) || e.fxt == FX_S3M_SPEED) && e.fxp != 0 {
        p.speed = e.fxp as i32;
    }
    if ((e.f2t == FX_SPEED && e.f2p < 0x20) || e.f2t == FX_S3M_SPEED) && e.f2p != 0 {
        p.speed = e.f2p as i32;
    }

    let xc = &mut p.xc_data[chn];

    /* Delay event read */
    let do_delay = if e.fxt == FX_EXTENDED && msn(e.fxp) == EX_DELAY && lsn(e.fxp) != 0 {
        xc.delay = lsn(e.fxp) + 1;
        true
    } else if e.f2t == FX_EXTENDED && msn(e.f2p) == EX_DELAY && lsn(e.f2p) != 0 {
        xc.delay = lsn(e.f2p) + 1;
        true
    } else if e.fxt == FX_MED_RETRIG && msn(e.fxp) != 0 {
        /* MED retrigger: reset retrigger so it doesn't continue during the delay. */
        xc.reset(RETRIG);
        xc.delay = msn(e.fxp) + 1;
        true
    } else if e.f2t == FX_MED_RETRIG && msn(e.f2p) != 0 {
        xc.reset(RETRIG);
        // FIXME (upstream): the delay of a MED retrigger in the second
        // effect slot is taken from the first slot's parameter.
        xc.delay = msn(e.fxp) + 1;
        true
    } else {
        false
    };

    if !do_delay {
        return 0;
    }

    // do_delay:
    xc.delayed_event = *e;

    if e.ins != 0 {
        xc.delayed_ins = e.ins as i32;
    }

    1
}

/// Translation of `read_row()`.
#[inline]
fn read_row(ctx: &mut ContextData, pat: i32, row: i32) {
    for chn in 0..ctx.m.mod_.chn {
        let num_rows = ctx.m.mod_.track(ctx.m.mod_.track_num(pat, chn)).rows;
        let event = if row < num_rows {
            *ctx.m.mod_.event(pat, chn, row)
        } else {
            XmpEvent::default()
        };

        if ctx.m.is_player_mode_ft2() {
            let xc = &mut ctx.p.xc_data[chn as usize];
            /* Reset Kxx, even if delayed (ft2_kxx.xm). */
            xc.keyoff = 0;
            /* Reset tremor, even if delayed (ft2_tremor_delay.xm). */
            xc.reset(TREMOR);
        }

        if check_delay(ctx, &event, chn as usize) == 0 {
            /* rowdelay_set bit 1 is set only in the first tick of the row
             * event if the delay causes the tick count resets to 0. We test
             * it to read row events only in the start of the row. (see the
             * OpenMPT test case FineVolColSlide.it)
             */
            let f = &ctx.p.flow;
            if f.rowdelay_set == 0
                || ((f.rowdelay_set & ROWDELAY_FIRST_FRAME != 0) && f.rowdelay > 0)
            {
                libxmp_read_event(ctx, &event, chn);
            }
        } else if ctx.m.is_player_mode_it() {
            /* Reset flags. See SlideDelay.it */
            ctx.p.xc_data[chn as usize].flags = 0;
        }
    }
}

/// Translation of `get_channel_vol()`.
#[inline]
fn get_channel_vol(ctx: &ContextData, chn: i32) -> i32 {
    let p = &ctx.p;

    /* channel is a root channel */
    if chn < p.virt.num_tracks {
        return p.channel_vol[chn as usize];
    }

    /* channel is invalid */
    if chn >= p.virt.virt_channels {
        return 0;
    }

    /* root is invalid */
    let root = libxmp_virt_getroot(ctx, chn);
    if root < 0 {
        return 0;
    }

    p.channel_vol[root as usize]
}

/// Translation of `tremor_ft2()`.
fn tremor_ft2(ctx: &mut ContextData, chn: usize, mut finalvol: i32) -> i32 {
    let frame = ctx.p.frame;
    let xc = &mut ctx.p.xc_data[chn];

    if xc.test(TREMOR) && frame != 0 {
        xc.tremor.count &= !TREMOR_SUPPRESS;
        if xc.tremor.count == 0 {
            /* end of down cycle, set up counter for up  */
            xc.tremor.count = xc.tremor.up | TREMOR_ON;
        } else if xc.tremor.count == TREMOR_ON {
            /* end of up cycle, set up counter for down */
            xc.tremor.count = xc.tremor.down;
        } else {
            xc.tremor.count = xc.tremor.count.wrapping_sub(1);
        }
    }

    if (xc.tremor.count & (TREMOR_ON | TREMOR_SUPPRESS)) == 0 {
        finalvol = 0;
    }

    finalvol
}

/// Translation of `tremor_s3m()`.
fn tremor_s3m(ctx: &mut ContextData, chn: usize, mut finalvol: i32) -> i32 {
    let xc = &mut ctx.p.xc_data[chn];

    if xc.test(TREMOR) {
        if xc.tremor.count == 0 {
            /* end of down cycle, set up counter for up  */
            xc.tremor.count = xc.tremor.up | TREMOR_ON;
        } else if xc.tremor.count == TREMOR_ON {
            /* end of up cycle, set up counter for down */
            xc.tremor.count = xc.tremor.down;
        }

        xc.tremor.count = xc.tremor.count.wrapping_sub(1);

        if !xc.tremor.count & TREMOR_ON != 0 {
            finalvol = 0;
        }
    }

    finalvol
}

/* Handle delayed keyoff effects. This should only be performed once on
 * the tick where Kxx activates (ft2_note_off_fade.xm).
 */
/// Translation of `delayed_keyoff()`.
fn delayed_keyoff(ctx: &mut ContextData, chn: usize) {
    let ins = ctx.p.xc_data[chn].ins;
    let aei_flg = libxmp_get_instrument(ctx, ins).map(|x| x.aei.flg);

    let p = &mut ctx.p;
    let frame = p.frame;
    let speed = p.speed;
    let xc = &mut p.xc_data[chn];

    match ctx.m.read_event_type {
        READ_EVENT_FT2 => {
            /* Ignore if frame>=speed (ft2_kxx.xm). */
            if frame >= speed {
                return;
            }
            /* See read_event_ft2 for more notes on keyoff. */
            if aei_flg.unwrap() & XMP_ENVELOPE_ON != 0 {
                xc.set_note(NOTE_RELEASE);
            } else {
                xc.volume = 0;
            }
            xc.set_note(NOTE_FADEOUT);
        }
        _ => {
            /* TODO: compatibility for old behavior (see process_volume)
             * until keyoff can be tested everywhere else.
             * Orpheus: keyoff clears the note, xx>speed works with delay.
             * RT2: keyoff clears the note, xx>speed acts like 0. */
            xc.set_note(NOTE_RELEASE);
        }
    }
}

/*
 * Update channel data
 */

/// `DOENV_RELEASE`
#[inline]
fn doenv_release(xc: &ChannelData, act: i32) -> i32 {
    (xc.test_note(NOTE_ENV_RELEASE) || act == VIRT_ACTION_OFF) as i32
}

/// Translation of `process_volume()`.
fn process_volume(ctx: &mut ContextData, chn: usize, act: i32) {
    let ins = ctx.p.xc_data[chn].ins;
    let (aei, ins_vol) = {
        let instrument = libxmp_get_instrument(ctx, ins).unwrap();
        (instrument.aei, instrument.vol)
    };
    let mut fade = 0;

    /* Keyoff and fadeout */

    /* Keyoff event in IT doesn't reset fadeout (see jeff93.it)
     * In XM it depends on envelope (see graff-strange_land.xm vs
     * Decibelter - Cosmic 'Wegian Mamas.xm)
     */
    {
        let xc = &mut ctx.p.xc_data[chn];
        if ctx.m.has_quirk(QUIRK_KEYOFF) {
            /* If IT, only apply fadeout on note release if we don't
             * have envelope, or if we have envelope loop
             */
            if (xc.test_note(NOTE_ENV_RELEASE) || act == VIRT_ACTION_OFF)
                && ((!aei.flg & XMP_ENVELOPE_ON != 0) || (aei.flg & XMP_ENVELOPE_LOOP != 0))
            {
                fade = 1;
            }
        } else if !ctx.m.is_player_mode_ft2() {
            /* TODO: FT2 doesn't do this. check other formats. */
            if !aei.flg & XMP_ENVELOPE_ON != 0 && xc.test_note(NOTE_ENV_RELEASE) {
                xc.fadeout = 0;
            }

            if xc.test_note(NOTE_ENV_RELEASE) || act == VIRT_ACTION_OFF {
                fade = 1;
            }
        }

        if !xc.test_per(VENV_PAUSE) {
            xc.v_idx = update_envelope(
                &ctx.m,
                &aei,
                xc.v_idx,
                doenv_release(xc, act),
                xc.test(KEY_OFF) as i32,
            );
        }

        let vol_envelope = get_envelope(&aei, xc.v_idx, 64) as u16;
        if check_envelope_end(&aei, xc.v_idx) != 0 {
            if vol_envelope == 0 {
                xc.set_note(NOTE_END);
            }
            xc.set_note(NOTE_ENV_END);
        }

        /* IT starts fadeout automatically at the end of the volume envelope. */
        match check_envelope_fade(&aei, xc.v_idx) {
            -1 => {
                xc.set_note(NOTE_END);
                /* Don't reset channel, we may have a tone portamento later
                 * virt_resetchannel(ctx, chn);
                 */
            }
            0 => {}
            _ => {
                if ctx.m.has_quirk(QUIRK_ENVFADE) {
                    xc.set_note(NOTE_FADEOUT);
                }
            }
        }

        /* IT envelope fadeout starts immediately after the envelope tick,
         * so process fadeout after the volume envelope. */
        if xc.test_note(NOTE_FADEOUT) || act == VIRT_ACTION_FADE {
            fade = 1;
        }

        if fade != 0 {
            if xc.fadeout > xc.ins_fade {
                xc.fadeout -= xc.ins_fade;
            } else {
                xc.fadeout = 0;
                xc.set_note(NOTE_END);
            }
        }

        /* If note ended in background channel, we can safely reset it */
        if xc.test_note(NOTE_END) && chn as i32 >= ctx.p.virt.num_tracks {
            libxmp_virt_resetchannel(ctx, chn as i32);
            return;
        }

        let _ = vol_envelope;
    }

    let vol_envelope = get_envelope(&aei, ctx.p.xc_data[chn].v_idx, 64) as u16;

    let mut finalvol = libxmp_extras_get_volume(ctx, chn);

    if ctx.m.is_player_mode_it() {
        let xc = &ctx.p.xc_data[chn];
        finalvol = xc.volume * (100 - xc.rvv) / 100;
    }

    if ctx.p.xc_data[chn].test(TREMOLO) {
        /* OpenMPT VibratoReset.mod */
        if !is_first_frame(ctx) || !ctx.m.has_quirk(QUIRK_PROTRACK) {
            let lfo = ctx.p.xc_data[chn].tremolo.lfo;
            finalvol += libxmp_lfo_get(ctx.m.read_event_type, &mut ctx.rng, &lfo, 0) / (1 << 6);
        }

        if !is_first_frame(ctx) || ctx.m.has_quirk(QUIRK_VIBALL) {
            libxmp_lfo_update(&mut ctx.p.xc_data[chn].tremolo.lfo);
        }
    }

    let m = &ctx.m;
    clamp(&mut finalvol, 0, m.volbase);

    {
        let xc = &ctx.p.xc_data[chn];
        let p = &ctx.p;
        finalvol = (finalvol.wrapping_mul(xc.fadeout)) >> 6; /* 16 bit output */

        finalvol = ((vol_envelope as i32)
            .wrapping_mul(p.gvol)
            .wrapping_mul(xc.mastervol)
            / m.gvolbase)
            .wrapping_mul(finalvol * 0x40 / m.volbase) as u32 as i32;
        finalvol = ((finalvol as u32) >> 18) as i32;
    }

    /* Apply channel volume */
    finalvol = finalvol * get_channel_vol(ctx, chn as i32) / 100;

    /* Volume translation table (for PTM, ARCH, COCO) */
    // Note (upstream): the index isn't checked against the table's
    // size; past it (only with volumes beyond their bases) this reads 0.
    if let Some(vol_table) = m.vol_table {
        finalvol = if m.volbase == 0xff {
            vol_table
                .get((finalvol >> 2) as usize)
                .copied()
                .unwrap_or(0)
                << 2
        } else {
            vol_table
                .get((finalvol >> 4) as usize)
                .copied()
                .unwrap_or(0)
                << 4
        };
    }

    if m.has_quirk(QUIRK_INSVOL) {
        finalvol = (finalvol * ins_vol * ctx.p.xc_data[chn].gvl) >> 12;
    }

    if ctx.m.is_player_mode_ft2() {
        finalvol = tremor_ft2(ctx, chn, finalvol);
    } else {
        finalvol = tremor_s3m(ctx, chn, finalvol);
    }
    ctx.p.xc_data[chn].macro_.finalvol = finalvol;

    if (chn as i32) < ctx.m.mod_.chn {
        finalvol = finalvol * ctx.p.master_vol / 100;
    } else {
        finalvol = finalvol * ctx.p.smix_vol / 100;
    }

    let xc = &mut ctx.p.xc_data[chn];
    xc.info_finalvol = if xc.test_note(NOTE_SAMPLE_END) {
        0
    } else {
        finalvol
    };
    let split = xc.split;
    let pair = xc.pair;

    libxmp_virt_setvol(ctx, chn as i32, finalvol);

    /* Check Amiga split channel */
    if split != 0 {
        libxmp_virt_setvol(ctx, pair as i32, finalvol);
    }
}

/// Translation of `process_frequency()`.
fn process_frequency(ctx: &mut ContextData, chn: usize, act: i32) {
    let ins = ctx.p.xc_data[chn].ins;
    let fei = libxmp_get_instrument(ctx, ins).unwrap().fei;

    {
        let xc = &mut ctx.p.xc_data[chn];
        if !xc.test_per(FENV_PAUSE) {
            xc.f_idx = update_envelope(
                &ctx.m,
                &fei,
                xc.f_idx,
                doenv_release(xc, act),
                xc.test(KEY_OFF) as i32,
            );
        }
    }
    let frq_envelope = get_envelope(&fei, ctx.p.xc_data[chn].f_idx, 0);

    /* Do note slide */

    if ctx.p.xc_data[chn].test(NOTE_SLIDE) {
        if ctx.p.xc_data[chn].noteslide.count == 0 {
            let (note, finetune, per_adj) = {
                let xc = &mut ctx.p.xc_data[chn];
                xc.note += xc.noteslide.slide;
                (xc.note, xc.finetune, xc.per_adj)
            };
            let period = libxmp_note_to_period(ctx, note, finetune, per_adj);
            let xc = &mut ctx.p.xc_data[chn];
            xc.period = period;
            xc.noteslide.count = xc.noteslide.speed;
        }
        ctx.p.xc_data[chn].noteslide.count -= 1;

        let note = ctx.p.xc_data[chn].note;
        libxmp_virt_setnote(ctx, chn as i32, note);
    }

    /* Instrument vibrato */
    let mut vibrato = {
        let xc = &ctx.p.xc_data[chn];
        let lfo = xc.insvib.lfo;
        1.0 * libxmp_lfo_get(ctx.m.read_event_type, &mut ctx.rng, &lfo, 1) as f64
            / (4096 * (1 + xc.insvib.sweep)) as f64
    };
    {
        let xc = &mut ctx.p.xc_data[chn];
        libxmp_lfo_update(&mut xc.insvib.lfo);
        if xc.insvib.sweep > 1 {
            xc.insvib.sweep -= 2;
        } else {
            xc.insvib.sweep = 0;
        }
    }

    /* Vibrato */
    if ctx.p.xc_data[chn].test(VIBRATO) || ctx.p.xc_data[chn].test_per(VIBRATO) {
        /* OpenMPT VibratoReset.mod */
        if !is_first_frame(ctx) || !ctx.m.has_quirk(QUIRK_PROTRACK) {
            let shift = if ctx.m.has_quirk(QUIRK_VIBHALF) {
                10
            } else {
                9
            };
            let lfo = ctx.p.xc_data[chn].vibrato.lfo;
            let vib = libxmp_lfo_get(ctx.m.read_event_type, &mut ctx.rng, &lfo, 1) / (1 << shift);

            if ctx.m.has_quirk(QUIRK_VIBINV) {
                vibrato -= vib as f64;
            } else {
                vibrato += vib as f64;
            }
        }

        if !is_first_frame(ctx) || ctx.m.has_quirk(QUIRK_VIBALL) {
            libxmp_lfo_update(&mut ctx.p.xc_data[chn].vibrato.lfo);
        }
    }

    let mut period = ctx.p.xc_data[chn].period;
    period += libxmp_extras_get_period(ctx, chn) as f64;

    if ctx.m.has_quirk(QUIRK_ST3BUGS) && period < 0.25 {
        libxmp_virt_resetchannel(ctx, chn as i32);
    }
    /* Sanity check */
    if period < 0.1 {
        period = 0.1;
    }

    /* Arpeggio */
    let mut arp = arpeggio(ctx, chn);

    /* Pitch bend */

    let (note, per_adj, finetune) = {
        let xc = &ctx.p.xc_data[chn];
        (xc.note, xc.per_adj, xc.finetune)
    };
    let mut linear_bend = libxmp_period_to_bend(ctx, period + vibrato, note, per_adj);

    {
        let xc = &ctx.p.xc_data[chn];
        if xc.test_note(NOTE_GLISSANDO) && xc.test(TONEPORTA) {
            if linear_bend > 0 {
                linear_bend = (linear_bend + 6400) / 12800 * 12800;
            } else if linear_bend < 0 {
                linear_bend = (linear_bend - 6400) / 12800 * 12800;
            }
        }
    }

    if ctx.m.has_quirk(QUIRK_FT2BUGS) && arp != 0 {
        /* OpenMPT ArpSlide.xm */
        linear_bend = linear_bend / 12800 * 12800 + finetune * 100;

        /* OpenMPT ArpeggioClamp.xm */
        if note + arp > 107 && ctx.p.speed - (ctx.p.frame % ctx.p.speed) > 0 {
            arp = 108 - note;
        }
    }

    /* Envelope */

    if ctx.p.xc_data[chn].f_idx >= 0 && (!fei.flg & XMP_ENVELOPE_FLT != 0) {
        /* IT pitch envelopes are always linear, even in Amiga period
         * mode. Each unit in the envelope scale is 1/25 semitone.
         */
        linear_bend += frq_envelope * 128;
    }

    /* Arpeggio */

    if arp != 0 {
        linear_bend += (100 << 7) * arp;

        /* OpenMPT ArpWrapAround.mod */
        if ctx.m.has_quirk(QUIRK_PROTRACK) {
            if note + arp > MAX_NOTE_MOD + 1 {
                linear_bend -= 12800 * (3 * 12);
            } else if note + arp > MAX_NOTE_MOD {
                libxmp_virt_setvol(ctx, chn as i32, 0);
            }
        }
    }

    linear_bend += libxmp_extras_get_linear_bend(ctx, chn);

    let mut final_period = libxmp_note_to_period_mix(note, linear_bend);

    /* From OpenMPT PeriodLimit.s3m:
     * "ScreamTracker 3 limits the final output period to be at least 64,
     *  i.e. when playing a note that is too high or when sliding the
     *  period lower than 64, the output period will simply be clamped to
     *  64. However, when reaching a period of 0 through slides, the
     *  output on the channel should be stopped."
     */
    /* ST3 uses periods*4, so the limit is 16. Adjusted to the exact
     * A6 value because we compute periods in floating point.
     */
    if ctx.m.has_quirk(QUIRK_ST3BUGS) && final_period < 16.239270 {
        /* A6 */
        final_period = 16.239270;
    }

    libxmp_virt_setperiod(ctx, chn as i32, final_period);

    /* For xmp_get_frame_info() */
    {
        let xc = &mut ctx.p.xc_data[chn];
        xc.info_pitchbend = linear_bend >> 7;
        xc.info_period = (final_period * 4096.0).min(i32::MAX as f64) as i32;
    }

    if ctx.m.is_period_modrng() {
        let min_period = libxmp_note_to_period(ctx, MAX_NOTE_MOD, finetune, 0.0) * 4096.0;
        let max_period = libxmp_note_to_period(ctx, MIN_NOTE_MOD, finetune, 0.0) * 4096.0;
        // CLAMP(xc->info_period, min_period, max_period) (an int and doubles)
        let xc = &mut ctx.p.xc_data[chn];
        if (xc.info_period as f64) < min_period {
            xc.info_period = min_period as i32;
        } else if (xc.info_period as f64) > max_period {
            xc.info_period = max_period as i32;
        }
    } else if ctx.p.xc_data[chn].info_period < (1 << 12) {
        ctx.p.xc_data[chn].info_period = 1 << 12;
    }

    /* Process filter */

    if !ctx.m.has_quirk(QUIRK_FILTER) {
        return;
    }

    let (mut cutoff, resonance, can_disable);
    {
        let xc = &mut ctx.p.xc_data[chn];
        if xc.f_idx >= 0 && (fei.flg & XMP_ENVELOPE_FLT != 0) {
            if frq_envelope < 0xfe {
                xc.filter.envelope = frq_envelope;
            }
            cutoff = xc.filter.cutoff * xc.filter.envelope >> 8;
        } else {
            cutoff = xc.filter.cutoff;
        }
        resonance = xc.filter.resonance;
        can_disable = xc.filter.can_disable;
    }

    if cutoff > 0xff {
        cutoff = 0xff;
    }
    /* IT: cutoff 127 + resonance 0 turns off the filter, but this
     * is only applied when playing a new note without toneporta.
     * All other combinations take effect immediately.
     * See OpenMPT filter-reset.it, filter-reset-carry.it */
    if cutoff < 0xfe || resonance > 0 || can_disable != 0 {
        let (a0, b0, b1) = libxmp_filter_setup(ctx.s.freq, cutoff, resonance);
        libxmp_virt_seteffect(ctx, chn as i32, DSP_EFFECT_FILTER_A0, a0);
        libxmp_virt_seteffect(ctx, chn as i32, DSP_EFFECT_FILTER_B0, b0);
        libxmp_virt_seteffect(ctx, chn as i32, DSP_EFFECT_FILTER_B1, b1);
        libxmp_virt_seteffect(ctx, chn as i32, DSP_EFFECT_RESONANCE, resonance);
        libxmp_virt_seteffect(ctx, chn as i32, DSP_EFFECT_CUTOFF, cutoff);
        ctx.p.xc_data[chn].filter.can_disable = 0;
    }
}

/// Translation of `process_pan()`.
fn process_pan(ctx: &mut ContextData, chn: usize, act: i32) {
    let ins = ctx.p.xc_data[chn].ins;
    let pei = libxmp_get_instrument(ctx, ins).unwrap().pei;
    let mut panbrello = 0;

    {
        let xc = &mut ctx.p.xc_data[chn];
        if !xc.test_per(PENV_PAUSE) {
            xc.p_idx = update_envelope(
                &ctx.m,
                &pei,
                xc.p_idx,
                doenv_release(xc, act),
                xc.test(KEY_OFF) as i32,
            );
        }
    }
    let pan_envelope = get_envelope(&pei, ctx.p.xc_data[chn].p_idx, 32);

    if ctx.p.xc_data[chn].test(PANBRELLO) {
        let lfo = ctx.p.xc_data[chn].panbrello.lfo;
        panbrello = libxmp_lfo_get(ctx.m.read_event_type, &mut ctx.rng, &lfo, 0) / 512;
        if is_first_frame(ctx) {
            libxmp_lfo_update(&mut ctx.p.xc_data[chn].panbrello.lfo);
        }
    }
    let s = &ctx.s;
    let is_it = ctx.m.is_player_mode_it();
    let xc = &mut ctx.p.xc_data[chn];
    xc.macro_.notepan = xc.pan.val + panbrello + 0x80;

    let channel_pan = xc.pan.val;

    let mut finalpan =
        channel_pan + panbrello + (pan_envelope - 32) * (128 - (xc.pan.val - 128).abs()) / 32;

    if is_it {
        finalpan += xc.rpv * 4;
    }

    clamp(&mut finalpan, 0, 255);

    if s.format & XMP_FORMAT_MONO != 0 || xc.pan.surround != 0 {
        finalpan = 0;
    } else {
        finalpan = (finalpan - 0x80) * s.mix / 100;
    }

    xc.info_finalpan = finalpan + 0x80;

    if xc.pan.surround != 0 {
        libxmp_virt_setpan(ctx, chn as i32, PAN_SURROUND);
    } else {
        libxmp_virt_setpan(ctx, chn as i32, finalpan);
    }
}

/// Translation of `update_volume()`.
fn update_volume(ctx: &mut ContextData, chn: usize) {
    let m = &ctx.m;
    let p = &mut ctx.p;
    let frame = p.frame;
    let speed = p.speed;
    let rowdelay_set = p.flow.rowdelay_set;
    let xc = &mut p.xc_data[chn];

    /* Volume slides happen in all frames but the first, except when the
     * "volume slide on all frames" flag is set.
     */
    if frame % speed != 0 || m.has_quirk(QUIRK_VSALL) {
        if xc.test(GVOL_SLIDE) {
            p.gvol += xc.gvol.slide;
        }

        if xc.test(VOL_SLIDE) || xc.test_per(VOL_SLIDE) {
            xc.volume += xc.vol.slide;
        }

        if xc.test_per(VOL_SLIDE) {
            if xc.vol.slide > 0 {
                let target = (xc.vol.target - 1).max(m.volbase);
                if xc.volume > target {
                    xc.volume = target;
                    xc.reset_per(VOL_SLIDE);
                }
            }
            if xc.vol.slide < 0 {
                let target = if xc.vol.target > 0 {
                    0.min(xc.vol.target - 1)
                } else {
                    0
                };
                if xc.volume < target {
                    xc.volume = target;
                    xc.reset_per(VOL_SLIDE);
                }
            }
        }

        if xc.test(VOL_SLIDE_2) {
            xc.volume += xc.vol.slide2;
        }
        if xc.test(TRK_VSLIDE) {
            xc.mastervol += xc.trackvol.slide;
        }
    }

    if frame % speed == 0 {
        /* Process "fine" effects */
        if xc.test(FINE_VOLS) {
            xc.volume += xc.vol.fslide;
        }

        if xc.test(FINE_VOLS_2) {
            /* OpenMPT FineVolColSlide.it:
             * Unlike fine volume slides in the effect column,
             * fine volume slides in the volume column are only
             * ever executed on the first tick -- not on multiples
             * of the first tick if there is a pattern delay.
             */
            if rowdelay_set == 0 || rowdelay_set & ROWDELAY_FIRST_FRAME != 0 {
                xc.volume += xc.vol.fslide2;
            }
        }

        if xc.test(TRK_FVSLIDE) {
            xc.mastervol += xc.trackvol.fslide;
        }

        if xc.test(GVOL_SLIDE) {
            p.gvol += xc.gvol.fslide;
        }
    }

    /* Clamp volumes */
    clamp(&mut xc.volume, 0, m.volbase);
    let volume = xc.volume;
    clamp(&mut xc.mastervol, 0, m.volbase);
    let split = xc.split;
    let pair = xc.pair;
    clamp(&mut p.gvol, 0, m.gvolbase);

    if split != 0 {
        p.xc_data[pair as usize].volume = volume;
    }
}

/// Translation of `update_frequency()`.
fn update_frequency(ctx: &mut ContextData, chn: usize) {
    let first = is_first_frame(ctx);

    if !first || ctx.m.has_quirk(QUIRK_PBALL) {
        let protrack = ctx.m.has_quirk(QUIRK_PROTRACK);
        let xc = &mut ctx.p.xc_data[chn];
        if xc.test(PITCHBEND) || xc.test_per(PITCHBEND) {
            xc.period += xc.freq.slide as f64;
            if protrack {
                xc.porta.target = xc.period;
            }
        }

        /* Do tone portamento */
        if (xc.test(TONEPORTA) || xc.test_per(TONEPORTA)) && xc.porta.target > 0.0 {
            let mut end = 0;
            if xc.porta.dir > 0 {
                xc.period += xc.porta.slide as f64;
                if xc.period >= xc.porta.target {
                    end = 1;
                }
            } else {
                xc.period -= xc.porta.slide as f64;
                if xc.period <= xc.porta.target {
                    end = 1;
                }
            }

            if end != 0 {
                /* reached end */
                xc.period = xc.porta.target;
                xc.porta.dir = 0;
                xc.reset(TONEPORTA);
                xc.reset_per(TONEPORTA);

                if protrack {
                    xc.porta.target = -1.0;
                }
            }
        }
    }

    if first {
        let xc = &mut ctx.p.xc_data[chn];
        if xc.test(FINE_BEND) {
            xc.period += xc.freq.fslide;
        }

        if xc.test(FINE_NSLIDE) {
            xc.note += xc.noteslide.fslide;
            let (note, finetune, per_adj) = (xc.note, xc.finetune, xc.per_adj);
            let period = libxmp_note_to_period(ctx, note, finetune, per_adj);
            ctx.p.xc_data[chn].period = period;
        }
    }

    match ctx.m.period_type {
        PERIOD_LINEAR => {
            let xc = &mut ctx.p.xc_data[chn];
            clamp(&mut xc.period, MIN_PERIOD_L as f64, MAX_PERIOD_L as f64);
        }
        PERIOD_MODRNG => {
            let finetune = ctx.p.xc_data[chn].finetune;
            let min_period = libxmp_note_to_period(ctx, MAX_NOTE_MOD, finetune, 0.0);
            let max_period = libxmp_note_to_period(ctx, MIN_NOTE_MOD, finetune, 0.0);
            clamp(&mut ctx.p.xc_data[chn].period, min_period, max_period);
        }
        _ => {}
    }

    /* Check for invalid periods (from Toru Egashira's NSPmod)
     * panic.s3m has negative periods
     * ambio.it uses low (~8) period values
     */
    if ctx.p.xc_data[chn].period < 0.25 {
        libxmp_virt_setvol(ctx, chn as i32, 0);
    }
}

/// Translation of `update_pan()`.
fn update_pan(ctx: &mut ContextData, chn: usize) {
    let first = is_first_frame(ctx);
    let xc = &mut ctx.p.xc_data[chn];

    if xc.test(PAN_SLIDE) {
        if first {
            xc.pan.val += xc.pan.fslide;
        } else {
            xc.pan.val += xc.pan.slide;
        }

        if xc.pan.val < 0 {
            xc.pan.val = 0;
        } else if xc.pan.val > 0xff {
            xc.pan.val = 0xff;
        }
    }
}

/// Translation of `play_channel()`.
fn play_channel(ctx: &mut ContextData, chn: usize) {
    ctx.p.xc_data[chn].info_finalvol = 0;

    /* IT tempo slide */
    if !is_first_frame(ctx) && ctx.p.xc_data[chn].test(TEMPO_SLIDE) {
        ctx.p.bpm += ctx.p.xc_data[chn].tempo_slide;
        clamp(&mut ctx.p.bpm, 0x20, 0xff);
    }

    /* Do delay */
    if ctx.p.xc_data[chn].delay > 0 {
        ctx.p.xc_data[chn].delay -= 1;
        if ctx.p.xc_data[chn].delay == 0 {
            let e = ctx.p.xc_data[chn].delayed_event;
            libxmp_read_event(ctx, &e, chn as i32);
        }
    }

    /* IT MIDI macros need to update regardless of the current voice state. */
    update_midi_macro(ctx, chn as i32);

    let act = libxmp_virt_cstat(ctx, chn as i32);
    if act == VIRT_INVALID {
        /* We need this to keep processing global volume slides */
        update_volume(ctx, chn);
        return;
    }

    let ins = ctx.p.xc_data[chn].ins;
    if ctx.p.frame == 0
        && act != VIRT_ACTIVE
        && (!is_valid_instrument_or_sfx(&ctx.m.mod_, &ctx.smix, ins) || act == VIRT_ACTION_CUT)
    {
        libxmp_virt_resetchannel(ctx, chn as i32);
        return;
    }

    if !is_valid_instrument_or_sfx(&ctx.m.mod_, &ctx.smix, ins) {
        return;
    }

    libxmp_play_extras(ctx, chn);

    /* Do cut/retrig */
    if ctx.p.xc_data[chn].test(RETRIG) {
        let s3mrtg = ctx.m.has_quirk(QUIRK_S3MRTG);
        let xc = &mut ctx.p.xc_data[chn];
        xc.retrig.count -= 1;
        let cond = if s3mrtg {
            xc.retrig.count <= 0
        } else {
            xc.retrig.count == 0
        };

        if cond {
            if xc.retrig.type_ < 0x10 {
                /* don't retrig on cut */
                libxmp_virt_voicepos(ctx, chn as i32, 0.0);
                /* Retrigger increases hold counter */
                libxmp_med_hold_retrigger(ctx, chn);
            } else {
                ctx.p.xc_data[chn].set_note(NOTE_END);
            }
            let xc = &mut ctx.p.xc_data[chn];
            let r = &RVAL[xc.retrig.type_ as usize];
            xc.volume += r.s;
            xc.volume *= r.m;
            xc.volume /= r.d;
            xc.retrig.count = xc.retrig.val;

            if xc.retrig.limit > 0 {
                /* Limit the number of retriggers. */
                xc.retrig.limit -= 1;
                if xc.retrig.limit == 0 {
                    xc.reset(RETRIG);
                }
            }
        }
    }

    /* Do keyoff */
    if ctx.p.xc_data[chn].keyoff != 0 {
        ctx.p.xc_data[chn].keyoff -= 1;
        if ctx.p.xc_data[chn].keyoff == 0 {
            delayed_keyoff(ctx, chn);
        }
    }

    let rel = ctx.p.xc_data[chn].test_note(NOTE_SAMPLE_RELEASE) as i32;
    libxmp_virt_release(ctx, chn as i32, rel);

    update_volume(ctx, chn);
    update_frequency(ctx, chn);
    update_pan(ctx, chn);

    process_volume(ctx, chn, act);
    process_frequency(ctx, chn, act);
    process_pan(ctx, chn, act);

    if ctx.m.has_quirk(QUIRK_PROTRACK | QUIRK_INVLOOP) && ctx.p.xc_data[chn].ins < ctx.m.mod_.ins {
        update_invloop(ctx, chn);
    }

    if ctx.p.xc_data[chn].test_note(NOTE_SUSEXIT) {
        ctx.p.xc_data[chn].set_note(NOTE_ENV_RELEASE);
    }

    ctx.p.xc_data[chn].info_position = libxmp_virt_getvoicepos(ctx, chn as i32) as i32;
}

/*
 * Event injection
 */

/// Translation of `inject_event()`.
fn inject_event(ctx: &mut ContextData) {
    for chn in 0..(ctx.m.mod_.chn + ctx.smix.chn) as usize {
        let e = ctx.p.inject_event[chn];
        if e._flag > 0 {
            libxmp_read_event(ctx, &e, chn as i32);
            ctx.p.inject_event[chn]._flag = 0;
        }
    }
}

/*
 * Sequencing
 */

/// Translation of `skip_invalid_orders()`.
fn skip_invalid_orders(ctx: &mut ContextData) -> i32 {
    let m = &ctx.m;
    let mod_ = &m.mod_;
    let p = &mut ctx.p;

    if p.ord < 0 {
        p.ord = 0;
    }

    loop {
        if p.ord >= mod_.len
            || (m.has_quirk(QUIRK_MARKER) && mod_.xxo[p.ord as usize] == XMP_MARK_END)
        {
            return -1;
        }
        if (mod_.xxo[p.ord as usize] as i32) < mod_.pat {
            break;
        }
        p.ord += 1;
    }
    0
}

/// Translation of `next_order()`.
fn next_order(ctx: &mut ContextData, mut last_ord: i32) -> i32 {
    ctx.p.ord += 1;

    if skip_invalid_orders(ctx) < 0 {
        let mod_ = &ctx.m.mod_;
        let entry_point = ctx.m.seq_data[ctx.p.sequence as usize].entry_point;
        /* Restart module */
        if mod_.rst > mod_.len || mod_.xxo(mod_.rst) >= mod_.pat || ctx.p.ord < entry_point {
            ctx.p.ord = entry_point;
        } else if libxmp_get_sequence(ctx, mod_.rst) == ctx.p.sequence {
            ctx.p.ord = mod_.rst;
        } else {
            ctx.p.ord = entry_point;
        }
        /* Nothing valid, even from the entry point? Fail.
         * This should be prevented by the scan, but check anyway. */
        if skip_invalid_orders(ctx) < 0 {
            ctx.p.bad_sequence = 1;
            return -1;
        }
        ctx.p.gvol = ctx.m.xxo_info[ctx.p.ord as usize].gvl;

        /* Module restart should always reset the play time. */
        last_ord = -1;
    }
    let m = &ctx.m;
    let mod_ = &m.mod_;
    let p = &mut ctx.p;
    /* Playback now has a valid position. */
    p.bad_sequence = 0;

    /* Bxx+Dxx within same position, Archimedes line jump,
     * etc. should not reset time tracking. */
    if last_ord != p.ord {
        p.current_time = m.xxo_info[p.ord as usize].time;
    }

    let f = &mut p.flow;
    f.num_rows = mod_.pattern(mod_.xxo[p.ord as usize] as i32).rows;
    if f.jumpline >= f.num_rows {
        f.jumpline = 0;
    }
    p.row = f.jumpline;
    f.jumpline = 0;

    p.pos = p.ord;
    p.frame = 0;

    /* Scream Tracker 3, Imago Orpheus: position change resets loop vars.
     * For some reason the pattern jump effect does not do this in IMF. */
    if m.has_flow_mode(FLOW_LOOP_PATTERN_RESET) {
        f.loop_start = -1;
        f.loop_count = 0;
        for i in 0..mod_.chn as usize {
            f.loop_[i].start = 0;
            f.loop_[i].count = 0;
        }
    }

    /* Reset persistent effects at new pattern */
    if m.has_quirk(QUIRK_PERPAT) {
        for chn in 0..mod_.chn as usize {
            p.xc_data[chn].per_flags = 0;
        }
    }
    0
}

/// Translation of `next_row()`.
fn next_row(ctx: &mut ContextData) -> i32 {
    let p = &mut ctx.p;
    let last_ord = p.ord;

    p.frame = 0;
    p.flow.delay = 0;
    p.flow.loop_param = -1;

    if p.flow.pbreak != 0 {
        p.flow.pbreak = 0;
        p.flow.loop_dest = -1;

        if p.flow.jump != -1 {
            p.ord = p.flow.jump - 1;
            p.flow.jump = -1;
        }

        return next_order(ctx, last_ord);
    } else {
        if p.flow.rowdelay == 0 {
            p.row += 1;
            p.flow.rowdelay_set = 0;
        } else {
            p.flow.rowdelay -= 1;
        }

        if p.flow.loop_dest >= 0 {
            p.row = p.flow.loop_dest;
            p.flow.loop_dest = -1;
        }

        /* check end of pattern */
        if p.row >= p.flow.num_rows {
            return next_order(ctx, last_ord);
        }
    }
    0
}

/*
 * Set note action for libxmp_virt_pastnote
 */
/// Translation of `libxmp_player_set_release()`.
pub(crate) fn libxmp_player_set_release(ctx: &mut ContextData, chn: i32) {
    ctx.p.xc_data[chn as usize].set_note(NOTE_RELEASE);
}

/// Translation of `libxmp_player_set_fadeout()`.
pub(crate) fn libxmp_player_set_fadeout(ctx: &mut ContextData, chn: i32) {
    ctx.p.xc_data[chn as usize].set_note(NOTE_FADEOUT);
}

/* Get frame time for calculation of the current playback time
 * based on the most recent scan. This value should be used for
 * playback time calculation ONLY. */
/// Translation of `libxmp_get_frame_time()`.
pub(crate) fn libxmp_get_frame_time(ctx: &ContextData) -> f64 {
    let p = &ctx.p;
    let m = &ctx.m;
    if p.bpm == 0 {
        return 0.0;
    }
    p.scan_time_factor * m.rrate / p.bpm as f64
}

/// Translation of `update_from_ord_info()`.
fn update_from_ord_info(ctx: &mut ContextData) {
    let p = &mut ctx.p;
    let oinfo = &ctx.m.xxo_info[p.ord as usize];

    if oinfo.speed != 0 {
        p.speed = oinfo.speed;
    }
    p.bpm = oinfo.bpm;
    p.gvol = oinfo.gvl;
    p.current_time = oinfo.time;

    p.st26_speed = oinfo.st26_speed;
}

/// Translation of `libxmp_reset_flow()`.
pub(crate) fn libxmp_reset_flow(ctx: &mut ContextData) {
    let f = &mut ctx.p.flow;
    f.jumpline = 0;
    f.jump = -1;
    f.pbreak = 0;
    f.loop_dest = -1;
    f.loop_param = -1;
    f.loop_start = -1;
    f.loop_count = 0;
    f.loop_active_num = 0;
    f.delay = 0;
    f.rowdelay = 0;
    f.rowdelay_set = 0;
    f.force_reposition = 0;

    if !f.loop_.is_empty() {
        for i in 0..ctx.m.mod_.chn as usize {
            f.loop_[i].start = 0;
            f.loop_[i].count = 0;
        }
    }
}

/// Translation of `xmp_start_player()`.
pub(crate) fn xmp_start_player(ctx: &mut ContextData, rate: i32, format: i32) -> i32 {
    if rate < XMP_MIN_SRATE || rate > XMP_MAX_SRATE {
        return -XMP_ERROR_INVALID;
    }

    if ctx.state < XMP_STATE_LOADED {
        return -XMP_ERROR_STATE;
    }

    if ctx.state > XMP_STATE_LOADED {
        xmp_end_player(ctx);
    }

    let c4rate = ctx.m.c4rate;
    if libxmp_mixer_on(ctx, rate, format, c4rate) < 0 {
        return -XMP_ERROR_INTERNAL;
    }

    {
        let m = &ctx.m;
        let mod_ = &m.mod_;
        let p = &mut ctx.p;
        p.time_factor_relative = 1.0;
        p.master_vol = 100;
        p.smix_vol = 100;
        p.gvol = m.volbase;
        p.ord = 0;
        p.pos = 0;
        p.frame = -1;
        p.row = 0;
        p.current_time = 0.0;
        p.loop_count = 0;
        p.sequence = 0;
        p.bad_sequence = 0;

        /* Set default volume and mute status */
        for i in 0..XMP_MAX_CHANNELS {
            if (i as i32) < mod_.chn && (mod_.xxc[i].flg & XMP_CHANNEL_MUTE != 0) {
                p.channel_mute[i] = 1;
            } else {
                p.channel_mute[i] = 0;
            }
            p.channel_vol[i] = 100;
        }
    }

    /* Handle markers/invalid patterns at start (the seventh laboratory.it).
     * If there are no valid orders or if an end marker is found, set some
     * safe default values (note: this previously zeroed mod->len). */
    if skip_invalid_orders(ctx) < 0 {
        /* Note: previously did this for mod->chn == 0, which caused
         * crashes on invalid order 0s. 0 channel modules are technically
         * valid (if useless) so just let them play normally. */
        let p = &mut ctx.p;
        p.ord = 0;
        p.scan[0].ord = 0;
        p.row = 0;
        p.scan[0].row = 0;
        p.bad_sequence = 1;
        p.flow.end_point = 0;
        p.flow.num_rows = 0;
    } else {
        let mod_ = &ctx.m.mod_;
        let p = &mut ctx.p;
        p.flow.num_rows = mod_.pattern(mod_.xxo[p.ord as usize] as i32).rows;
        p.flow.end_point = p.scan[0].num;
    }

    update_from_ord_info(ctx);

    let num = ctx.m.mod_.chn + ctx.smix.chn;
    if libxmp_virt_on(ctx, num) != 0 {
        return -XMP_ERROR_INTERNAL;
    }

    libxmp_reset_flow(ctx);

    let n = ctx.p.virt.virt_channels.max(0) as usize;
    let Ok(loop_) = try_vec(n, PatternLoop::default()) else {
        return -XMP_ERROR_SYSTEM;
    };
    ctx.p.flow.loop_ = loop_;

    let Ok(xc_data) = try_vec(n, ChannelData::default()) else {
        ctx.p.flow.loop_ = Vec::new();
        return -XMP_ERROR_SYSTEM;
    };
    ctx.p.xc_data = xc_data;

    /* Reset our buffer pointers */
    xmp_play_buffer(ctx, None, 0, 0);

    for i in 0..n {
        ctx.p.xc_data[i].filter.cutoff = 0xff;
        let mut xc = std::mem::take(&mut ctx.p.xc_data[i]);
        let ret = libxmp_new_channel_extras(ctx, &mut xc);
        ctx.p.xc_data[i] = xc;
        if ret < 0 {
            ctx.p.xc_data = Vec::new();
            ctx.p.flow.loop_ = Vec::new();
            return -XMP_ERROR_INTERNAL;
        }
    }
    reset_channels(ctx);

    ctx.state = XMP_STATE_PLAYING;

    0
}

/// Translation of `check_end_of_module()`.
fn check_end_of_module(ctx: &mut ContextData) {
    let p = &mut ctx.p;
    let seq = p.sequence as usize;

    /* check end of module */
    if p.ord == p.scan[seq].ord && p.row == p.scan[seq].row {
        if p.flow.end_point == 0 {
            p.loop_count += 1;
            p.flow.end_point = p.scan[seq].num;
            /* return -1; */
        }
        p.flow.end_point -= 1;
    }
}

/// Translation of `xmp_play_frame()`.
pub(crate) fn xmp_play_frame(ctx: &mut ContextData) -> i32 {
    if ctx.state < XMP_STATE_PLAYING {
        return -XMP_ERROR_STATE;
    }

    if ctx.m.mod_.len <= 0 {
        return -XMP_END;
    }

    /* check reposition */
    if ctx.p.ord != ctx.p.pos || ctx.p.flow.force_reposition != 0 {
        let p = &mut ctx.p;
        let start = ctx.m.seq_data[p.sequence as usize].entry_point;
        p.flow.force_reposition = 0;

        if p.pos == -2 {
            /* set by xmp_module_stop */
            return -XMP_END; /* that's all folks */
        }

        if p.pos == -1 {
            /* restart sequence */
            p.pos = start;
        }

        let seq = p.sequence as usize;
        if p.pos == start {
            p.flow.end_point = p.scan[seq].num;
        }

        /* Check if lands after a loop point */
        if p.pos > p.scan[seq].ord {
            p.flow.end_point = 0;
        }

        p.flow.jump = -1;

        p.ord = p.pos - 1;

        /* Stay inside our subsong */
        if p.ord < start {
            p.ord = start - 1;
        }

        if next_order(ctx, -1) < 0 {
            /* Current sequence contains no valid orders;
             * playback can't continue. */
            return -XMP_END;
        }

        update_from_ord_info(ctx);

        libxmp_virt_reset(ctx);
        reset_channels(ctx);
    } else {
        /* Sequences with no valid orders can't continue playing from
         * here and require manual repositioning to another sequence. */
        if ctx.p.bad_sequence != 0 {
            return -XMP_END;
        }

        ctx.p.frame += 1;
        if ctx.p.frame >= (ctx.p.speed * (1 + ctx.p.flow.delay)) {
            /* If break during pattern delay, next row is skipped.
             * See corruption.mod order 1D (pattern 0D) last line:
             * EE2 + D31 ignores D00 in order 1C line 31. Reported
             * by The Welder <welder@majesty.net>, Jan 14 2012
             */
            if ctx.m.has_quirk(QUIRK_PROTRACK) && ctx.p.flow.delay != 0 && ctx.p.flow.pbreak != 0 {
                if next_row(ctx) < 0 {
                    return -XMP_END;
                }
                check_end_of_module(ctx);
            }
            if next_row(ctx) < 0 {
                return -XMP_END;
            }
        }
    }

    for i in 0..ctx.m.mod_.chn as usize {
        ctx.p.xc_data[i].reset(KEY_OFF);
    }

    /* check new row */

    if ctx.p.frame == 0 {
        /* first frame in row */
        check_end_of_module(ctx);
        let pat = ctx.m.mod_.xxo[ctx.p.ord as usize] as i32;
        let row = ctx.p.row;
        read_row(ctx, pat, row);

        let p = &mut ctx.p;
        if p.st26_speed != 0 {
            if p.st26_speed & 0x10000 != 0 {
                p.speed = (p.st26_speed & 0xff00) >> 8;
            } else {
                p.speed = p.st26_speed & 0xff;
            }
            p.st26_speed ^= 0x10000;
        }
    }

    inject_event(ctx);

    /* play_frame */
    for i in 0..ctx.p.virt.virt_channels as usize {
        play_channel(ctx, i);
    }

    ctx.p.flow.rowdelay_set &= !ROWDELAY_FIRST_FRAME;

    ctx.p.current_time += libxmp_get_frame_time(ctx);

    libxmp_mixer_softmixer(ctx);

    0
}

/// Translation of `xmp_play_buffer()`: fill `out_buffer` (or, with
/// `None`, reset the internal state).
pub(crate) fn xmp_play_buffer(
    ctx: &mut ContextData,
    out_buffer: Option<&mut [u8]>,
    size: i32,
    loop_: i32,
) -> i32 {
    let mut ret = 0;
    let mut filled = 0;

    /* Reset internal state
     * Syncs buffer start with frame start */
    let Some(out_buffer) = out_buffer else {
        let p = &mut ctx.p;
        p.loop_count = 0;
        p.buffer_data.consumed = 0;
        p.buffer_data.in_size = 0;
        return 0;
    };

    if ctx.state < XMP_STATE_PLAYING {
        return -XMP_ERROR_STATE;
    }

    let mut fi = XmpFrameInfo::default();

    /* Fill buffer */
    while filled < size {
        /* Check if buffer full */
        if ctx.p.buffer_data.consumed == ctx.p.buffer_data.in_size {
            ret = xmp_play_frame(ctx);
            xmp_get_frame_info(ctx, &mut fi);

            /* Check end of module */
            if ret < 0 || (loop_ > 0 && fi.loop_count >= loop_) {
                /* Start of frame, return end of replay */
                if filled == 0 {
                    ctx.p.buffer_data.consumed = 0;
                    ctx.p.buffer_data.in_size = 0;
                    return -1;
                }

                /* Fill remaining of this buffer */
                out_buffer[filled as usize..size as usize].fill(0);
                return 0;
            }

            ctx.p.buffer_data.consumed = 0;
            ctx.p.buffer_data.in_size = fi.buffer_size;
        }

        /* Copy frame data to user buffer */
        let bd = ctx.p.buffer_data;
        let copy_size = (size - filled).min(bd.in_size - bd.consumed);
        out_buffer[filled as usize..(filled + copy_size) as usize].copy_from_slice(
            &ctx.s.buffer[bd.consumed as usize..(bd.consumed + copy_size) as usize],
        );
        ctx.p.buffer_data.consumed += copy_size;
        filled += copy_size;
    }

    ret
}

/// Translation of `xmp_end_player()`.
pub(crate) fn xmp_end_player(ctx: &mut ContextData) {
    if ctx.state < XMP_STATE_PLAYING {
        return;
    }

    ctx.state = XMP_STATE_LOADED;

    /* Free channel extras */
    for i in 0..ctx.p.virt.virt_channels as usize {
        let mut xc = std::mem::take(&mut ctx.p.xc_data[i]);
        libxmp_release_channel_extras(ctx, &mut xc);
        ctx.p.xc_data[i] = xc;
    }

    libxmp_virt_off(ctx);

    ctx.p.xc_data = Vec::new();
    ctx.p.flow.loop_ = Vec::new();

    libxmp_mixer_off(ctx);
}

/// Translation of `xmp_get_module_info()` (`None` before a module is
/// loaded, where upstream leaves `info` as it was).
pub(crate) fn xmp_get_module_info(ctx: &ContextData) -> Option<XmpModuleInfo<'_>> {
    let m = &ctx.m;
    let mod_ = &m.mod_;

    if ctx.state < XMP_STATE_LOADED {
        return None;
    }

    Some(XmpModuleInfo {
        md5: m.md5,
        mod_,
        comment: m.comment.as_deref(),
        num_sequences: m.num_sequences,
        seq_data: &m.seq_data,
        vol_base: m.volbase,
    })
}

/// Translation of `xmp_get_frame_info()`.
pub(crate) fn xmp_get_frame_info(ctx: &ContextData, info: &mut XmpFrameInfo) {
    let p = &ctx.p;
    let s = &ctx.s;
    let m = &ctx.m;
    let mod_ = &m.mod_;

    if ctx.state < XMP_STATE_LOADED {
        return;
    }

    let chn = mod_.chn;

    if p.pos >= 0 && p.pos < mod_.len {
        info.pos = p.pos;
    } else {
        info.pos = 0;
    }

    info.pattern = mod_.xxo[info.pos as usize] as i32;

    if info.pattern < mod_.pat {
        info.num_rows = mod_.pattern(info.pattern).rows;
    } else {
        info.num_rows = 0;
    }

    /* API still uses integers for time... */
    let mut current_time = p.current_time;
    let mut total_time = p.scan[p.sequence as usize].time;
    clamp(&mut current_time, 0.0, i32::MAX as f64);
    clamp(&mut total_time, 0.0, i32::MAX as f64);

    info.row = p.row;
    info.frame = p.frame;
    info.speed = p.speed;
    info.bpm = p.bpm;
    info.total_time = total_time as i32;
    info.frame_time = (libxmp_get_frame_time(ctx) * 1000.0) as i32;
    info.time = current_time as i32;

    info.total_size = s.total_size;
    info.buffer_size = s.ticksize * s.output_chn * s.sample_size;

    info.volume = p.gvol;
    info.loop_count = p.loop_count;
    info.virt_channels = p.virt.virt_channels;
    info.virt_used = p.virt.virt_used;

    info.sequence = p.sequence;

    if !p.xc_data.is_empty() {
        for i in 0..chn as usize {
            let c = &p.xc_data[i];
            let ci = &mut info.channel_info[i];

            ci.note = c.key as u8;
            ci.pitchbend = c.info_pitchbend as i16;
            ci.period = c.info_period as u32;
            ci.position = c.info_position as u32;
            ci.instrument = c.ins as u8;
            ci.sample = c.smp as u8;
            ci.volume = (c.info_finalvol >> 4) as u8;
            ci.pan = c.info_finalpan as u8;
            ci.reserved = 0;
            ci.event = XmpEvent::default();

            if info.pattern < mod_.pat && info.row < info.num_rows {
                let trk = mod_.pattern(info.pattern).index[i];
                let track = mod_.track(trk);
                if info.row < track.rows {
                    ci.event = track.event[info.row as usize];
                }
            }
        }
    }
}
