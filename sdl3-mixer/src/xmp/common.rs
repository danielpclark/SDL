// Rust translation of src/common.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The library's state (`struct context_data` and its parts), constants,
//! quirk flags and helper macros shared by the player and the loaders.

use super::extras::ModuleExtras;
use super::mixer::MixerVoice;
use super::player::ChannelData;
use super::*;

/* Constants */
pub(crate) const PAL_RATE: f64 = 250.0; /* 1 / (50Hz * 80us)		  */
pub(crate) const NTSC_RATE: f64 = 208.0; /* 1 / (60Hz * 80us)		  */
pub(crate) const C4_PAL_RATE: i32 = 8287; /* 7093789.2 / period (C4) * 2	  */
pub(crate) const C4_NTSC_RATE: i32 = 8363; /* 7159090.5 / period (C4) * 2	  */

/* [Amiga] PAL color carrier frequency (PCCF) = 4.43361825 MHz */
/* [Amiga] CPU clock = 1.6 * PCCF = 7.0937892 MHz */

pub(crate) const DEFAULT_AMPLIFY: i32 = 1;
pub(crate) const DEFAULT_MIX: i32 = 100;

/// `MSN(x)`
#[inline]
pub(crate) fn msn<T: Into<i32>>(x: T) -> i32 {
    (x.into() & 0xf0) >> 4
}

/// `LSN(x)`
#[inline]
pub(crate) fn lsn<T: Into<i32>>(x: T) -> i32 {
    x.into() & 0x0f
}

/* libxmp_get_filetype() return values */
pub(crate) const XMP_FILETYPE_NONE: i32 = 0;
pub(crate) const XMP_FILETYPE_DIR: i32 = 1 << 0;
pub(crate) const XMP_FILETYPE_FILE: i32 = 1 << 1;

/// `CLAMP(x, a, b)`
#[inline]
pub(crate) fn clamp<T: PartialOrd>(x: &mut T, a: T, b: T) {
    if *x < a {
        *x = a;
    } else if *x > b {
        *x = b;
    }
}

/// `XMP_ASL(x, e)`: standards-compliant sign-extending arithmetic left shift.
#[inline]
pub(crate) fn xmp_asl(x: i32, e: u32) -> i32 {
    ((x as u32) << e) as i32
}

/// `XMP_ASL64(x, e)`
#[inline]
pub(crate) fn xmp_asl64(x: i64, e: u32) -> i64 {
    ((x as u64) << e) as i64
}

/* Output file size limit for files unpacked from unarchivers into RAM. Most
 * general archive compression formats can't nicely bound the output size
 * from their input filesize, and a cap is needed for a few reasons:
 *
 * - Linux is too dumb for its own good and its malloc/realloc will return
 *   pointers to RAM that doesn't exist instead of NULL. When these are used,
 *   it will kill the application instead of allowing it to fail gracefully.
 * - libFuzzer and the clang sanitizers have malloc/realloc interceptors that
 *   terminate with an error instead of returning NULL.
 *
 * Depackers that have better ways of bounding the output size can ignore this.
 * This value is fairly arbitrary and can be changed if needed.
 */
pub(crate) const LIBXMP_DEPACK_LIMIT: i64 = 512 << 20;

/* Quirks */
pub(crate) const QUIRK_S3MLOOP: i32 = 1 << 0; /* S3M loop mode */
pub(crate) const QUIRK_ENVFADE: i32 = 1 << 1; /* Fade at end of envelope */
pub(crate) const QUIRK_PROTRACK: i32 = 1 << 2; /* Use Protracker-specific quirks */
pub(crate) const QUIRK_RTONCE: i32 = 1 << 3; /* Retrigger one time only */
pub(crate) const QUIRK_ST3BUGS: i32 = 1 << 4; /* Scream Tracker 3 bug compatibility */
pub(crate) const QUIRK_FINEFX: i32 = 1 << 5; /* Enable 0xf/0xe for fine effects */
pub(crate) const QUIRK_VSALL: i32 = 1 << 6; /* Volume slides in all frames */
pub(crate) const QUIRK_PBALL: i32 = 1 << 7; /* Pitch bending in all frames */
pub(crate) const QUIRK_PERPAT: i32 = 1 << 8; /* Cancel persistent fx at pat start */
pub(crate) const QUIRK_VOLPDN: i32 = 1 << 9; /* Set priority to volume slide down */
pub(crate) const QUIRK_UNISLD: i32 = 1 << 10; /* Unified pitch slide/portamento */
pub(crate) const QUIRK_ITVPOR: i32 = 1 << 11; /* Disable fine bends in IT vol fx */
/*#define QUIRK_FTMOD	(1 << 12)*/
/* Flag for multichannel mods */
pub(crate) const QUIRK_INVLOOP: i32 = 1 << 13; /* Enable invert loop */
/*#define QUIRK_MODRNG	(1 << 13)*/
/* Limit periods to MOD range */
pub(crate) const QUIRK_INSVOL: i32 = 1 << 14; /* Use instrument volume */
pub(crate) const QUIRK_VIRTUAL: i32 = 1 << 15; /* Enable virtual channels */
pub(crate) const QUIRK_FILTER: i32 = 1 << 16; /* Enable filter */
pub(crate) const QUIRK_IGSTPOR: i32 = 1 << 17; /* Ignore stray tone portamento */
pub(crate) const QUIRK_KEYOFF: i32 = 1 << 18; /* Keyoff doesn't reset fadeout */
pub(crate) const QUIRK_VIBHALF: i32 = 1 << 19; /* Vibrato is half as deep */
pub(crate) const QUIRK_VIBALL: i32 = 1 << 20; /* Vibrato in all frames */
pub(crate) const QUIRK_VIBINV: i32 = 1 << 21; /* Vibrato has inverse waveform */
pub(crate) const QUIRK_PRENV: i32 = 1 << 22; /* Portamento resets envelope & fade */
pub(crate) const QUIRK_ITOLDFX: i32 = 1 << 23; /* IT old effects mode */
pub(crate) const QUIRK_S3MRTG: i32 = 1 << 24; /* S3M-style retrig when count == 0 */
pub(crate) const QUIRK_FT2ENV: i32 = 1 << 25; /* Use FT2-style envelope handling */
pub(crate) const QUIRK_FT2BUGS: i32 = 1 << 26; /* FT2 bug compatibility */
pub(crate) const QUIRK_MARKER: i32 = 1 << 27; /* Patterns 0xfe and 0xff reserved */
pub(crate) const QUIRK_NOBPM: i32 = 1 << 28; /* Adjust speed only, no BPM */
pub(crate) const QUIRK_ARPMEM: i32 = 1 << 29; /* Arpeggio has memory (S3M_ARPEGGIO) */
pub(crate) const QUIRK_RSTCHN: i32 = 1 << 30; /* Reset channel on sample end */

/* Format quirks */
pub(crate) const QUIRKS_ST3: i32 =
    QUIRK_S3MLOOP | QUIRK_VOLPDN | QUIRK_FINEFX | QUIRK_S3MRTG | QUIRK_MARKER | QUIRK_RSTCHN;
pub(crate) const QUIRKS_FT2: i32 = 0;
pub(crate) const QUIRKS_IT: i32 = QUIRK_S3MLOOP
    | QUIRK_FINEFX
    | QUIRK_VIBALL
    | QUIRK_ENVFADE
    | QUIRK_ITVPOR
    | QUIRK_KEYOFF
    | QUIRK_VIRTUAL
    | QUIRK_FILTER
    | QUIRK_RSTCHN
    | QUIRK_IGSTPOR
    | QUIRK_S3MRTG
    | QUIRK_MARKER;

/* Quirks specific to flow effects, especially Pattern Loop. */
pub(crate) const FLOW_LOOP_GLOBAL_TARGET: i32 = 1 << 0; /* Global target for all tracks */
pub(crate) const FLOW_LOOP_GLOBAL_COUNT: i32 = 1 << 1; /* Global count for all tracks */
pub(crate) const FLOW_LOOP_END_ADVANCES: i32 = 1 << 2; /* Loop end advances target (S3M) */
pub(crate) const FLOW_LOOP_END_CANCELS: i32 = 1 << 3; /* Loop end cancels prev jumps on row (LIQ) */
pub(crate) const FLOW_LOOP_PATTERN_RESET: i32 = 1 << 4; /* Target/count reset on pattern change */
pub(crate) const FLOW_LOOP_INIT_SAMEROW: i32 = 1 << 5; /* SBx sets target if it isn't set (ST 3.01) */
pub(crate) const FLOW_LOOP_FIRST_EFFECT: i32 = 1 << 6; /* Only execute the first E60/E6x in a row */
pub(crate) const FLOW_LOOP_ONE_AT_A_TIME: i32 = 1 << 7; /* Init E6x if no other channel is looping (MPT) */
pub(crate) const FLOW_LOOP_IGNORE_TARGET: i32 = 1 << 8; /* Ignore E60 if count is >=1 (LIQ) */
pub(crate) const FLOW_LOOP_DELAY_BREAK: i32 = 1 << 9; /* E6x jump prevents later Dxx on same row (S3M, IT) */
pub(crate) const FLOW_LOOP_DELAY_JUMP: i32 = 1 << 10; /* E6x jump prevents later Bxx on same row (S3M) */
pub(crate) const FLOW_LOOP_UNSET_BREAK: i32 = 1 << 11; /* E6x jump cancels prior Dxx on same row (S3M, IMF) */
pub(crate) const FLOW_LOOP_UNSET_JUMP: i32 = 1 << 12; /* E6x jump cancels prior Bxx on same row (S3M) */
pub(crate) const FLOW_LOOP_SHARED_BREAK: i32 = 1 << 13; /* E6x overrides prior Dxx dest on same row (LIQ) */
/*#define FLOW_LOOP_TICK_0_JUMP */
/* Loop jump shortens row to one tick (DTM) */
pub(crate) const FLOW_JUMP_THEN_BREAK: i32 = 1 << 28; /* TODO: Bxx Dxx jumps, then breaks (IMF, TT) */
pub(crate) const FLOW_JUMP_QUEUED: i32 = 1 << 29; /* TODO: Jump queues next position (ST2) */
pub(crate) const FLOW_JUMP_NO_ROW_SET: i32 = 1 << 30; /* Jump doesn't set break row to 0 (ST3/IT) */

pub(crate) const FLOW_MODE_GENERIC: i32 = 0;
pub(crate) const FLOW_LOOP_GLOBAL: i32 = FLOW_LOOP_GLOBAL_TARGET | FLOW_LOOP_GLOBAL_COUNT;
/* Simulate players where all breaks and jumps are ignored during a loop jump. */
pub(crate) const FLOW_LOOP_NO_BREAK_JUMP: i32 =
    FLOW_LOOP_DELAY_BREAK | FLOW_LOOP_DELAY_JUMP | FLOW_LOOP_UNSET_BREAK | FLOW_LOOP_UNSET_JUMP;

/* Scream Tracker 3. No S3Ms seem to rely on the earlier behavior mode.
 * 3.01b has a bug where the end advancement sets the target to the same line
 * instead of the next line; there's no way to make use of this without getting
 * stuck, so it's not simulated.
 * ST2 has an entirely different behavior for pattern jump where, instead of
 * immediately jumping, it queues the next position to be loaded after a break
 * or the end of the current pattern.
 */
pub(crate) const FLOW_MODE_ST2: i32 = FLOW_JUMP_QUEUED;
pub(crate) const FLOW_MODE_ST3_301: i32 = FLOW_LOOP_GLOBAL
    | FLOW_LOOP_PATTERN_RESET
    | FLOW_LOOP_END_ADVANCES
    | FLOW_LOOP_INIT_SAMEROW
    | FLOW_JUMP_NO_ROW_SET;
pub(crate) const FLOW_MODE_ST3_321: i32 = FLOW_LOOP_GLOBAL
    | FLOW_LOOP_PATTERN_RESET
    | FLOW_LOOP_END_ADVANCES
    | FLOW_LOOP_NO_BREAK_JUMP
    | FLOW_JUMP_NO_ROW_SET;

/* Impulse Tracker. Not clear if anything relies on the old behavior types.
 * IT loops were global pre-1.04, and loop jumps override any prior break/jump.
 * IT 2.00+ loop jumps also delay any following break, but not pattern jumps.
 * IT 2.10+ reintroduced ST3's loop target advancement.
 */
pub(crate) const FLOW_MODE_IT_100: i32 =
    FLOW_LOOP_GLOBAL | FLOW_LOOP_UNSET_BREAK | FLOW_LOOP_UNSET_JUMP | FLOW_JUMP_NO_ROW_SET;
pub(crate) const FLOW_MODE_IT_104: i32 =
    FLOW_LOOP_UNSET_BREAK | FLOW_LOOP_UNSET_JUMP | FLOW_JUMP_NO_ROW_SET;
pub(crate) const FLOW_MODE_IT_200: i32 = FLOW_MODE_IT_104 | FLOW_LOOP_DELAY_BREAK;
pub(crate) const FLOW_MODE_IT_210: i32 = FLOW_MODE_IT_200 | FLOW_LOOP_END_ADVANCES;

/* Modplug Tracker/early OpenMPT */
pub(crate) const FLOW_MODE_MPT_116: i32 =
    FLOW_LOOP_ONE_AT_A_TIME | FLOW_LOOP_NO_BREAK_JUMP | FLOW_JUMP_NO_ROW_SET;

/* Imago Orpheus. Pattern Jump actually does not reset target/count, but all
 * other forms of pattern change do. Unclear if anything relies on it.
 * An XAx jump will set the destination row of a prior Txx jump.
 * An XAx jump will cancel a prior Uxx break on the same row.
 * Txx unsets a prior Uxx (normal); Txx followed by Uyy jumps and then
 * breaks to the next pattern after position X.
 */
pub(crate) const FLOW_MODE_ORPHEUS: i32 =
    FLOW_LOOP_PATTERN_RESET | FLOW_LOOP_SHARED_BREAK | FLOW_LOOP_UNSET_BREAK | FLOW_JUMP_THEN_BREAK;

/* Liquid Tracker uses generic MOD loops with an added behavior where
 * the end of a loop will cancel any other jump in the row that preceded it.
 * M60 is also ignored in channels that have started a loop for some reason.
 * When M6x jumps, it overrides any prior break line set by Jxx/Cxx.
 * There is also a "Scream Tracker" compatibility mode (only detectable in the
 * newer format) that adds LOOP_MODE_PATTERN_RESET.
 */
pub(crate) const FLOW_MODE_LIQUID: i32 =
    FLOW_LOOP_END_CANCELS | FLOW_LOOP_IGNORE_TARGET | FLOW_LOOP_SHARED_BREAK;
pub(crate) const FLOW_MODE_LIQUID_COMPAT: i32 = FLOW_MODE_LIQUID | FLOW_LOOP_PATTERN_RESET;

/* Octalyser (Atari). Looping jumps to the original position E60 was used in,
 * which libxmp doesn't simulate for now since it mostly gets the player stuck.
 * Octalyser ignores E60 if a loop is currently active; it's not clear if it's
 * possible for a module to actually rely on this behavior. Loop jumps
 * interrupt all breaks/jumps on the same row.
 *
 * LOOP_MODE_END_CANCELS is inaccurate but needed to fix "Dammed Illusion",
 * which has multiple E6x on one line that don't trigger because the module
 * expects to play in 4 channel mode. This quirk only works for this module
 * because it uses even loop counts, and doesn't break any other modules
 * because multiple E6x on a row otherwise traps the player.
 */
pub(crate) const FLOW_MODE_OCTALYSER: i32 =
    FLOW_LOOP_GLOBAL | FLOW_LOOP_IGNORE_TARGET | FLOW_LOOP_END_CANCELS | FLOW_LOOP_NO_BREAK_JUMP;

/* Digital Tracker prior to shareware 1.02 doesn't use LOOP_MODE_FIRST_EFFECT,
 * but any MOD that would rely on it is impossible to fingerprint.
 * Early versions had fully working loop jump precedence over jump/break;
 * later versions gradually break this in ways libxmp only partially implements.
 * Commercial version 1.9(?) added per-track counters.
 * Digital Home Studio added a bizarre tick-0 jump bug.
 */
pub(crate) const FLOW_MODE_DTM_2015: i32 =
    FLOW_LOOP_GLOBAL | FLOW_LOOP_FIRST_EFFECT | FLOW_LOOP_NO_BREAK_JUMP;
pub(crate) const FLOW_MODE_DTM_2015_6CH: i32 = FLOW_LOOP_GLOBAL
    | FLOW_LOOP_FIRST_EFFECT
    | FLOW_LOOP_DELAY_BREAK
    | FLOW_LOOP_UNSET_BREAK
    | FLOW_LOOP_SHARED_BREAK;
pub(crate) const FLOW_MODE_DTM_19: i32 =
    FLOW_LOOP_GLOBAL_TARGET | FLOW_LOOP_UNSET_BREAK | FLOW_LOOP_SHARED_BREAK;
// (FLOW_MODE_DTM_DHS uses FLOW_LOOP_TICK_0_JUMP, which isn't defined, and
// nothing uses it.)

/* DSP effects */
pub(crate) const DSP_EFFECT_CUTOFF: i32 = 0x02;
pub(crate) const DSP_EFFECT_RESONANCE: i32 = 0x03;
pub(crate) const DSP_EFFECT_FILTER_A0: i32 = 0xb0;
pub(crate) const DSP_EFFECT_FILTER_B0: i32 = 0xb1;
pub(crate) const DSP_EFFECT_FILTER_B1: i32 = 0xb2;

/* Time factor */
pub(crate) const DEFAULT_TIME_FACTOR: f64 = 10.0;
pub(crate) const MED_TIME_FACTOR: f64 = 2.64;
pub(crate) const FAR_TIME_FACTOR: f64 = 4.01373; /* See far_extras.c */

pub(crate) const NO_SEQUENCE: i32 = MAX_SEQUENCES as i32;
pub(crate) const MAX_SEQUENCES: usize = 255;
pub(crate) const MAX_SAMPLE_SIZE: i32 = 0x10000000;
pub(crate) const MAX_SAMPLES: i32 = 1024;
pub(crate) const MAX_INSTRUMENTS: i32 = 255;
pub(crate) const MAX_PATTERNS: i32 = 256;

pub(crate) const READ_EVENT_MOD: i32 = 0;
pub(crate) const READ_EVENT_FT2: i32 = 1;
pub(crate) const READ_EVENT_ST3: i32 = 2;
pub(crate) const READ_EVENT_IT: i32 = 3;
pub(crate) const READ_EVENT_MED: i32 = 4;

pub(crate) const PERIOD_AMIGA: i32 = 0;
pub(crate) const PERIOD_MODRNG: i32 = 1;
pub(crate) const PERIOD_LINEAR: i32 = 2;
pub(crate) const PERIOD_CSPD: i32 = 3;

/// Translation of `struct ord_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OrdData {
    pub time: f64, /* replay time (ms) at start of ord */
    pub speed: i32,
    pub bpm: i32,
    pub gvl: i32,
    pub start_row: i32,
    pub st26_speed: i32,
}

/* Context */

/// Translation of `struct smix_data`.
#[derive(Clone, Debug, Default)]
pub(crate) struct SmixData {
    pub chn: i32,
    pub ins: i32,
    pub smp: i32,
    pub xxi: Vec<XmpInstrument>,
    pub xxs: Vec<XmpSample>,
}

/* This will be added to the sample structure in the next API revision */
/// Translation of `struct extra_sample_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ExtraSampleData {
    pub c5spd: f64,
    pub sus: i32,
    pub sue: i32,
}

/// Translation of `struct midi_macro`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MidiMacro {
    pub data: [u8; 32],
}

/// Translation of `struct midi_macro_data`.
#[derive(Clone, Debug)]
pub(crate) struct MidiMacroData {
    pub param: [MidiMacro; 16],
    pub fixed: [MidiMacro; 128],
}

impl Default for MidiMacroData {
    fn default() -> Self {
        MidiMacroData {
            param: [MidiMacro { data: [0; 32] }; 16],
            fixed: [MidiMacro { data: [0; 32] }; 128],
        }
    }
}

/// Translation of `struct module_data`.
#[derive(Debug)]
pub(crate) struct ModuleData {
    pub mod_: XmpModule,

    pub dirname: Option<Vec<u8>>,          /* file dirname */
    pub basename: Option<Vec<u8>>,         /* file basename */
    pub filename: Option<Vec<u8>>,         /* Module file name */
    pub comment: Option<Vec<u8>>,          /* Comments, if any */
    pub md5: [u8; 16],                     /* MD5 message digest */
    pub size: i32,                         /* File size */
    pub rrate: f64,                        /* Replay rate */
    pub time_factor: f64,                  /* Time conversion constant */
    pub c4rate: i32,                       /* C4 replay rate */
    pub volbase: i32,                      /* Volume base */
    pub gvolbase: i32,                     /* Global volume base */
    pub gvol: i32,                         /* Global volume */
    pub mvolbase: i32,                     /* Mix volume base (S3M/IT) */
    pub mvol: i32,                         /* Mix volume (S3M/IT) */
    pub vol_table: Option<&'static [i32]>, /* Volume translation table */
    pub quirk: i32,                        /* player quirks */
    pub flow_mode: i32,                    /* Flow quirks, esp. Pattern Loop */
    pub read_event_type: i32,
    pub period_type: i32,
    pub smpctl: i32, /* sample control flags */
    pub defpan: i32, /* default pan setting */
    pub xxo_info: [OrdData; XMP_MAX_MOD_LENGTH],
    pub num_sequences: i32,
    pub seq_data: [XmpSequence; MAX_SEQUENCES],
    pub instrument_path: Option<Vec<u8>>,
    pub extra: ModuleExtras,    /* format-specific extra fields */
    pub scan_cnt: Vec<Vec<u8>>, /* scan counters */
    pub xtra: Vec<ExtraSampleData>,
    pub midi: Option<Box<MidiMacroData>>,
    pub compare_vblank: i32,
}

impl Default for ModuleData {
    fn default() -> Self {
        ModuleData {
            mod_: XmpModule::default(),
            dirname: None,
            basename: None,
            filename: None,
            comment: None,
            md5: [0; 16],
            size: 0,
            rrate: 0.0,
            time_factor: 0.0,
            c4rate: 0,
            volbase: 0,
            gvolbase: 0,
            gvol: 0,
            mvolbase: 0,
            mvol: 0,
            vol_table: None,
            quirk: 0,
            flow_mode: 0,
            read_event_type: 0,
            period_type: 0,
            smpctl: 0,
            defpan: 0,
            xxo_info: [OrdData::default(); XMP_MAX_MOD_LENGTH],
            num_sequences: 0,
            seq_data: [XmpSequence::default(); MAX_SEQUENCES],
            instrument_path: None,
            extra: ModuleExtras::None,
            scan_cnt: Vec::new(),
            xtra: Vec::new(),
            midi: None,
            compare_vblank: 0,
        }
    }
}

impl ModuleData {
    /// `HAS_QUIRK(x)`
    #[inline]
    pub fn has_quirk(&self, x: i32) -> bool {
        self.quirk & x != 0
    }

    /// `HAS_FLOW_MODE(x)`
    #[inline]
    pub fn has_flow_mode(&self, x: i32) -> bool {
        self.flow_mode & x != 0
    }

    /// `IS_PLAYER_MODE_MOD()`
    #[inline]
    pub fn is_player_mode_mod(&self) -> bool {
        self.read_event_type == READ_EVENT_MOD
    }
    /// `IS_PLAYER_MODE_FT2()`
    #[inline]
    pub fn is_player_mode_ft2(&self) -> bool {
        self.read_event_type == READ_EVENT_FT2
    }
    /// `IS_PLAYER_MODE_ST3()`
    #[inline]
    pub fn is_player_mode_st3(&self) -> bool {
        self.read_event_type == READ_EVENT_ST3
    }
    /// `IS_PLAYER_MODE_IT()`
    #[inline]
    pub fn is_player_mode_it(&self) -> bool {
        self.read_event_type == READ_EVENT_IT
    }
    /// `IS_PLAYER_MODE_MED()`
    #[inline]
    pub fn is_player_mode_med(&self) -> bool {
        self.read_event_type == READ_EVENT_MED
    }
    /// `IS_PERIOD_MODRNG()`
    #[inline]
    pub fn is_period_modrng(&self) -> bool {
        self.period_type == PERIOD_MODRNG
    }
    /// `IS_PERIOD_LINEAR()`
    #[inline]
    pub fn is_period_linear(&self) -> bool {
        self.period_type == PERIOD_LINEAR
    }
    /// `IS_PERIOD_CSPD()`
    #[inline]
    pub fn is_period_cspd(&self) -> bool {
        self.period_type == PERIOD_CSPD
    }
    /// `IS_AMIGA_MOD()`
    #[inline]
    pub fn is_amiga_mod(&self) -> bool {
        self.is_player_mode_mod() && self.is_period_modrng()
    }
}

/// Translation of `struct pattern_loop`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PatternLoop {
    pub start: i32,
    pub count: i32,
}

pub(crate) const ROWDELAY_ON: i32 = 1 << 0;
pub(crate) const ROWDELAY_FIRST_FRAME: i32 = 1 << 1;

/// Translation of `struct flow_control`.
#[derive(Clone, Debug, Default)]
pub(crate) struct FlowControl {
    pub pbreak: i32,
    pub jump: i32,
    pub delay: i32,
    pub jumpline: i32,
    pub loop_dest: i32,       /* Pattern loop destination, -1 for none */
    pub loop_param: i32,      /* Last loop param for Digital Tracker */
    pub loop_start: i32,      /* Global loop target for S3M et al. */
    pub loop_count: i32,      /* Global loop count for S3M et al. */
    pub loop_active_num: i32, /* Number of active loops for scan */

    pub loop_: Vec<PatternLoop>,

    pub num_rows: i32,
    pub end_point: i32,
    pub rowdelay: i32, /* For IT pattern row delay */
    pub rowdelay_set: i32,
    pub force_reposition: i32,
}

/// Translation of `struct virt_channel`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VirtChannel {
    pub count: i32,
    pub map: i32,
}

/// Translation of `struct scan_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ScanData {
    pub time: f64, /* replay time in ms */
    pub row: i32,
    pub ord: i32,
    pub num: i32,
}

/// Translation of `struct virt_control`.
#[derive(Debug, Default)]
pub(crate) struct VirtControl {
    pub num_tracks: i32,    /* Number of tracks */
    pub virt_channels: i32, /* Number of virtual channels */
    pub virt_used: i32,     /* Number of voices currently in use */
    pub maxvoc: i32,        /* Number of sound card voices */

    pub virt_channel: Vec<VirtChannel>,

    pub voice_array: Vec<MixerVoice>,
}

/// The `buffer_data` of `struct player_data` (`in_buffer` is the mixer's
/// buffer).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BufferData {
    pub consumed: i32,
    pub in_size: i32,
}

/// Translation of `struct player_data`.
#[derive(Debug)]
pub(crate) struct PlayerData {
    pub ord: i32,
    pub pos: i32,
    pub row: i32,
    pub frame: i32,
    pub speed: i32,
    pub bpm: i32,
    pub mode: i32,
    pub player_flags: i32,
    pub flags: i32,

    pub time_factor_relative: f64, /* User time factor adjustment */
    pub scan_time_factor: f64,     /* m->time_factor for most recent scan */
    pub current_time: f64,         /* current time based on scan time factor */

    pub loop_count: i32,
    pub sequence: i32,
    pub bad_sequence: i32, /* 1 = can't find next order, need reposition */
    pub sequence_control: [u8; XMP_MAX_MOD_LENGTH],

    pub smix_vol: i32,   /* SFX volume */
    pub master_vol: i32, /* Music volume */
    pub gvol: i32,

    pub flow: FlowControl,

    pub scan: Vec<ScanData>,

    pub xc_data: Vec<ChannelData>,

    pub channel_vol: [i32; XMP_MAX_CHANNELS],
    pub channel_mute: [i8; XMP_MAX_CHANNELS],

    pub virt: VirtControl,

    pub inject_event: [XmpEvent; XMP_MAX_CHANNELS],

    pub buffer_data: BufferData,

    pub st26_speed: i32, /* For IceTracker speed effect */
    pub filter: i32,     /* Amiga led filter */
}

impl Default for PlayerData {
    fn default() -> Self {
        PlayerData {
            ord: 0,
            pos: 0,
            row: 0,
            frame: 0,
            speed: 0,
            bpm: 0,
            mode: 0,
            player_flags: 0,
            flags: 0,
            time_factor_relative: 0.0,
            scan_time_factor: 0.0,
            current_time: 0.0,
            loop_count: 0,
            sequence: 0,
            bad_sequence: 0,
            sequence_control: [0; XMP_MAX_MOD_LENGTH],
            smix_vol: 0,
            master_vol: 0,
            gvol: 0,
            flow: FlowControl::default(),
            scan: Vec::new(),
            xc_data: Vec::new(),
            channel_vol: [0; XMP_MAX_CHANNELS],
            channel_mute: [0; XMP_MAX_CHANNELS],
            virt: VirtControl::default(),
            inject_event: [XmpEvent::default(); XMP_MAX_CHANNELS],
            buffer_data: BufferData::default(),
            st26_speed: 0,
            filter: 0,
        }
    }
}

/// Translation of `struct mixer_data`. `buffer` is the output buffer, in
/// bytes; `buf32` the temporary buffer for 32 bit samples.
#[derive(Debug, Default)]
pub(crate) struct MixerData {
    pub freq: i32,        /* sampling rate */
    pub format: i32,      /* sample format */
    pub amplify: i32,     /* amplification multiplier */
    pub mix: i32,         /* percentage of channel separation */
    pub interp: i32,      /* interpolation type */
    pub dsp: i32,         /* dsp effect flags */
    pub buffer: Vec<u8>,  /* output buffer */
    pub buf32: Vec<i32>,  /* temporary buffer for 32 bit samples */
    pub total_size: i32,  /* allocated samples (not frames) in buffers */
    pub sample_size: i32, /* individual output sample (not frame) size */
    pub output_chn: i32,  /* output channels (1 or 2) */
    pub numvoc: i32,      /* default softmixer voices number */
    pub ticksize: i32,
    pub dtright: i32,      /* anticlick control, right channel */
    pub dtleft: i32,       /* anticlick control, left channel */
    pub bidir_adjust: i32, /* adjustment for IT bidirectional loops */
    pub pbase: f64,        /* period base */
}

/// Translation of `struct rng_state`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RngState {
    pub state: u32,
}

/// Translation of `struct context_data`.
#[derive(Debug, Default)]
pub(crate) struct ContextData {
    pub p: PlayerData,
    pub s: MixerData,
    pub m: ModuleData,
    pub smix: SmixData,
    pub rng: RngState,
    pub state: i32,
}

/// `libxmp_strdup()`, for the byte strings here.
pub(crate) fn libxmp_strdup(src: Option<&[u8]>) -> Option<Vec<u8>> {
    src.map(|s| s.to_vec())
}

/// Allocate `len` copies of `v`, failing (instead of aborting) when the
/// memory can't be had, as upstream's `malloc()`/`calloc()` return `NULL`.
pub(crate) fn try_vec<T: Clone>(len: usize, v: T) -> Result<Vec<T>, ()> {
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| ())?;
    out.resize(len, v);
    Ok(out)
}

/// `isprint()` in the C locale.
#[inline]
pub(crate) fn isprint(c: u8) -> bool {
    (0x20..0x7f).contains(&c)
}

/// `isdigit()`
#[inline]
pub(crate) fn isdigit(c: u8) -> bool {
    c.is_ascii_digit()
}

/// `strlen()` of a NUL-terminated buffer.
#[inline]
pub(crate) fn strlen(s: &[u8]) -> usize {
    s.iter().position(|&c| c == 0).unwrap_or(s.len())
}

/// `strncpy(dst, src, n)`, where `src` ends at its first NUL or its end.
pub(crate) fn strncpy(dst: &mut [u8], src: &[u8], n: usize) {
    let src = &src[..strlen(src)];
    for i in 0..n {
        dst[i] = if i < src.len() { src[i] } else { 0 };
    }
}

/// `strncmp(a, b, n) == 0`, the strings ending at a NUL or at their ends.
pub(crate) fn strneq(a: &[u8], b: &[u8], n: usize) -> bool {
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return false;
        }
        if x == 0 {
            return true;
        }
    }
    true
}

/// C's `(int)` conversion of a `double`, as x86-64 does it: NaN and values
/// out of `int`'s range (undefined behavior in C) become `INT_MIN`, where a
/// Rust `as` would give 0 or saturate.
pub(crate) fn c_int(x: f64) -> i32 {
    if x.is_nan() || x >= 2147483648.0 || x <= -2147483649.0 {
        i32::MIN
    } else {
        x as i32
    }
}

/// `snprintf(dst, size, "%s", s)`: copy at most `size - 1` bytes and a NUL.
pub(crate) fn snprintf_bytes(dst: &mut [u8], size: usize, s: &[u8]) {
    if size == 0 {
        return;
    }
    let n = s.len().min(size - 1);
    dst[..n].copy_from_slice(&s[..n]);
    dst[n] = 0;
}
