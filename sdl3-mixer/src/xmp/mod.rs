// Rust translation of include/xmp.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! libxmp, the Extended Module Player library, translated: the module
//! loaders, the player and its software mixer, as SDL_mixer vendors and
//! builds it (the full library, not libxmp-lite, with
//! `LIBXMP_DISABLE_DEPACKERS` and `LIBXMP_DISABLE_PROWIZARD`), for the
//! `XMP` decoder (`decoder_xmp.rs`).
//!
//! libxmp is under the MIT license (see LICENSE.txt); `md5.rs` and
//! `loaders/itsex.rs` translate public-domain code, `miniz_tinfl.rs` is
//! under miniz's MIT license, and `loaders/lzw.rs`, `loaders/xmf_load.rs`
//! and `loaders/abk_load.rs` carry their own copyright lines.
//!
//! Only the API SDL_mixer's decoder calls, and what it reaches, is
//! translated: loading a module through callbacks, playing it frame by
//! frame and seeking (with, for the tests, loading from memory and the
//! player parameters). The file-path and `FILE *` loaders, the sound
//! effect player (`smix.c`, beyond what the player consults), event
//! injection, tempo factors, channel muting and the format list aren't,
//! nor are the depackers and the ProWizard loaders, which SDL_mixer's
//! build leaves out. All 52 loaders that build enables are translated, in
//! upstream's order ([`format`] lists them); the Magnetic Fields Packer
//! one only takes `FILE *` handles, so it never matches here, as it never
//! does under SDL_mixer.
//!
//! The C structures are kept, with indices where upstream keeps pointers
//! into them, and `Vec`s (allocated fallibly where a size comes from a
//! file) where it keeps arrays.

#![allow(dead_code)]
#![allow(non_upper_case_globals)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::needless_late_init)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::manual_range_patterns)]

pub(crate) mod common;
pub(crate) mod control;
pub(crate) mod dataio;
pub(crate) mod effects;
pub(crate) mod extras;
pub(crate) mod far_extras;
pub(crate) mod filetype;
pub(crate) mod filter;
pub(crate) mod flow;
pub(crate) mod flt_extras;
pub(crate) mod format;
pub(crate) mod hio;
pub(crate) mod hmn_extras;
pub(crate) mod lfo;
pub(crate) mod load;
pub(crate) mod load_helpers;
pub(crate) mod loaders;
pub(crate) mod md5;
pub(crate) mod med_extras;
pub(crate) mod miniz_tinfl;
pub(crate) mod mix_all;
pub(crate) mod mix_paula;
pub(crate) mod mixer;
pub(crate) mod path;
pub(crate) mod period;
pub(crate) mod player;
pub(crate) mod precomp_blep;
pub(crate) mod precomp_lut;
pub(crate) mod read_event;
pub(crate) mod rng;
pub(crate) mod scan;
pub(crate) mod smix;
pub(crate) mod virt;

pub(crate) const XMP_VERSION: &str = "4.7.3";
pub(crate) const XMP_VERCODE: u32 = 0x040703;
pub(crate) const XMP_VER_MAJOR: i32 = 4;
pub(crate) const XMP_VER_MINOR: i32 = 7;
pub(crate) const XMP_VER_RELEASE: i32 = 3;

pub(crate) const XMP_NAME_SIZE: usize = 64; /* Size of module name and type */

pub(crate) const XMP_KEY_OFF: u8 = 0x81; /* Note number for key off event */
pub(crate) const XMP_KEY_CUT: u8 = 0x82; /* Note number for key cut event */
pub(crate) const XMP_KEY_FADE: u8 = 0x83; /* Note number for fade event */

/* mixer parameter macros */

/* sample format flags */
pub(crate) const XMP_FORMAT_8BIT: i32 = 1 << 0; /* Mix to 8-bit instead of 16 */
pub(crate) const XMP_FORMAT_UNSIGNED: i32 = 1 << 1; /* Mix to unsigned samples */
pub(crate) const XMP_FORMAT_MONO: i32 = 1 << 2; /* Mix to mono instead of stereo */
pub(crate) const XMP_FORMAT_32BIT: i32 = 1 << 3; /* Mix to 32-bit int instead of 16 */

/* player parameters */
pub(crate) const XMP_PLAYER_AMP: i32 = 0; /* Amplification factor */
pub(crate) const XMP_PLAYER_MIX: i32 = 1; /* Stereo mixing */
pub(crate) const XMP_PLAYER_INTERP: i32 = 2; /* Interpolation type */
pub(crate) const XMP_PLAYER_DSP: i32 = 3; /* DSP effect flags */
pub(crate) const XMP_PLAYER_FLAGS: i32 = 4; /* Player flags */
pub(crate) const XMP_PLAYER_CFLAGS: i32 = 5; /* Player flags for current module */
pub(crate) const XMP_PLAYER_SMPCTL: i32 = 6; /* Sample control flags */
pub(crate) const XMP_PLAYER_VOLUME: i32 = 7; /* Player module volume */
pub(crate) const XMP_PLAYER_STATE: i32 = 8; /* Internal player state (read only) */
pub(crate) const XMP_PLAYER_SMIX_VOLUME: i32 = 9; /* SMIX volume */
pub(crate) const XMP_PLAYER_DEFPAN: i32 = 10; /* Default pan setting */
pub(crate) const XMP_PLAYER_MODE: i32 = 11; /* Player personality */
pub(crate) const XMP_PLAYER_MIXER_TYPE: i32 = 12; /* Current mixer (read only) */
pub(crate) const XMP_PLAYER_VOICES: i32 = 13; /* Maximum number of mixer voices */

/* interpolation types */
pub(crate) const XMP_INTERP_NEAREST: i32 = 0; /* Nearest neighbor */
pub(crate) const XMP_INTERP_LINEAR: i32 = 1; /* Linear (default) */
pub(crate) const XMP_INTERP_SPLINE: i32 = 2; /* Cubic spline */

/* dsp effect types */
pub(crate) const XMP_DSP_LOWPASS: i32 = 1 << 0; /* Lowpass filter effect */
pub(crate) const XMP_DSP_ALL: i32 = XMP_DSP_LOWPASS;

/* player state */
pub(crate) const XMP_STATE_UNLOADED: i32 = 0; /* Context created */
pub(crate) const XMP_STATE_LOADED: i32 = 1; /* Module loaded */
pub(crate) const XMP_STATE_PLAYING: i32 = 2; /* Module playing */

/* player flags */
pub(crate) const XMP_FLAGS_VBLANK: i32 = 1 << 0; /* Use vblank timing */
pub(crate) const XMP_FLAGS_FX9BUG: i32 = 1 << 1; /* Emulate FX9 bug */
pub(crate) const XMP_FLAGS_FIXLOOP: i32 = 1 << 2; /* Emulate sample loop bug */
pub(crate) const XMP_FLAGS_A500: i32 = 1 << 3; /* Use Paula mixer in Amiga modules */

/* player modes */
pub(crate) const XMP_MODE_AUTO: i32 = 0; /* Autodetect mode (default) */
pub(crate) const XMP_MODE_MOD: i32 = 1; /* Play as a generic MOD player */
pub(crate) const XMP_MODE_NOISETRACKER: i32 = 2; /* Play using Noisetracker quirks */
pub(crate) const XMP_MODE_PROTRACKER: i32 = 3; /* Play using Protracker quirks */
pub(crate) const XMP_MODE_S3M: i32 = 4; /* Play as a generic S3M player */
pub(crate) const XMP_MODE_ST3: i32 = 5; /* Play using ST3 bug emulation */
pub(crate) const XMP_MODE_ST3GUS: i32 = 6; /* Play using ST3+GUS quirks */
pub(crate) const XMP_MODE_XM: i32 = 7; /* Play as a generic XM player */
pub(crate) const XMP_MODE_FT2: i32 = 8; /* Play using FT2 bug emulation */
pub(crate) const XMP_MODE_IT: i32 = 9; /* Play using IT quirks */
pub(crate) const XMP_MODE_ITSMP: i32 = 10; /* Play using IT sample mode quirks */

/* mixer types */
pub(crate) const XMP_MIXER_STANDARD: i32 = 0; /* Standard mixer */
pub(crate) const XMP_MIXER_A500: i32 = 1; /* Amiga 500 */
pub(crate) const XMP_MIXER_A500F: i32 = 2; /* Amiga 500 with led filter */

/* sample flags */
pub(crate) const XMP_SMPCTL_SKIP: i32 = 1 << 0; /* Don't load samples */

/* limits */
pub(crate) const XMP_MAX_KEYS: usize = 121; /* Number of valid keys */
pub(crate) const XMP_MAX_ENV_POINTS: usize = 32; /* Max number of envelope points */
pub(crate) const XMP_MAX_MOD_LENGTH: usize = 256; /* Max number of patterns in module */
pub(crate) const XMP_MAX_CHANNELS: usize = 64; /* Max number of channels in module */
pub(crate) const XMP_MAX_SRATE: i32 = 768000; /* max sampling rate (Hz) */
pub(crate) const XMP_MIN_SRATE: i32 = 4000; /* min sampling rate (Hz) */
pub(crate) const XMP_MIN_BPM: i32 = 20; /* min BPM */
/* frame rate = (50 * bpm / 125) Hz */
/* frame size = (sampling rate * channels * size) / frame rate */
pub(crate) const XMP_MAX_FRAMESIZE: i32 = 5 * XMP_MAX_SRATE * 2 / XMP_MIN_BPM;

/* error codes */
pub(crate) const XMP_END: i32 = 1;
pub(crate) const XMP_ERROR_INTERNAL: i32 = 2; /* Internal error */
pub(crate) const XMP_ERROR_FORMAT: i32 = 3; /* Unsupported module format */
pub(crate) const XMP_ERROR_LOAD: i32 = 4; /* Error loading file */
pub(crate) const XMP_ERROR_DEPACK: i32 = 5; /* Error depacking file */
pub(crate) const XMP_ERROR_SYSTEM: i32 = 6; /* System error */
pub(crate) const XMP_ERROR_INVALID: i32 = 7; /* Invalid parameter */
pub(crate) const XMP_ERROR_STATE: i32 = 8; /* Invalid player state */

pub(crate) const XMP_CHANNEL_SYNTH: i32 = 1 << 0; /* Channel is synthesized */
pub(crate) const XMP_CHANNEL_MUTE: i32 = 1 << 1; /* Channel is muted */
pub(crate) const XMP_CHANNEL_SPLIT: i32 = 1 << 2; /* Split Amiga channel in bits 5-4 */
pub(crate) const XMP_CHANNEL_SURROUND: i32 = 1 << 4; /* Surround channel */

/// Translation of `struct xmp_channel`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XmpChannel {
    pub pan: i32, /* Channel pan (0x80 is center) */
    pub vol: i32, /* Channel volume */
    pub flg: i32, /* Channel flags */
}

/// Translation of `struct xmp_pattern` (its `index[1]` flexible array is
/// a `Vec` of `chn` track indices).
#[derive(Clone, Debug, Default)]
pub(crate) struct XmpPattern {
    pub rows: i32,       /* Number of rows */
    pub index: Vec<i32>, /* Track index */
}

/// Translation of `struct xmp_event`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct XmpEvent {
    pub note: u8,  /* Note number (0 means no note) */
    pub ins: u8,   /* Patch number */
    pub vol: u8,   /* Volume (0 to basevol) */
    pub fxt: u8,   /* Effect type */
    pub fxp: u8,   /* Effect parameter */
    pub f2t: u8,   /* Secondary effect type */
    pub f2p: u8,   /* Secondary effect parameter */
    pub _flag: u8, /* Internal (reserved) flags */
}

/// Translation of `struct xmp_track` (its `event[1]` flexible array is a
/// `Vec` of `rows` events).
#[derive(Clone, Debug, Default)]
pub(crate) struct XmpTrack {
    pub rows: i32,            /* Number of rows */
    pub event: Vec<XmpEvent>, /* Event data */
}

pub(crate) const XMP_ENVELOPE_ON: i32 = 1 << 0; /* Envelope is enabled */
pub(crate) const XMP_ENVELOPE_SUS: i32 = 1 << 1; /* Envelope has sustain point */
pub(crate) const XMP_ENVELOPE_LOOP: i32 = 1 << 2; /* Envelope has loop */
pub(crate) const XMP_ENVELOPE_FLT: i32 = 1 << 3; /* Envelope is used for filter */
pub(crate) const XMP_ENVELOPE_SLOOP: i32 = 1 << 4; /* Envelope has sustain loop */
pub(crate) const XMP_ENVELOPE_CARRY: i32 = 1 << 5; /* Don't reset envelope position */

/// Translation of `struct xmp_envelope`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct XmpEnvelope {
    pub flg: i32, /* Flags */
    pub npt: i32, /* Number of envelope points */
    pub scl: i32, /* Envelope scaling */
    pub sus: i32, /* Sustain start point */
    pub sue: i32, /* Sustain end point */
    pub lps: i32, /* Loop start point */
    pub lpe: i32, /* Loop end point */
    pub data: [i16; XMP_MAX_ENV_POINTS * 2],
}

impl Default for XmpEnvelope {
    fn default() -> Self {
        XmpEnvelope {
            flg: 0,
            npt: 0,
            scl: 0,
            sus: 0,
            sue: 0,
            lps: 0,
            lpe: 0,
            data: [0; XMP_MAX_ENV_POINTS * 2],
        }
    }
}

impl XmpEnvelope {
    /// `env->data[i]` (the loaders and `check_envelope()` keep the
    /// points an enabled envelope uses in the array).
    #[inline]
    pub fn d(&self, i: i32) -> i32 {
        self.data[i as usize] as i32
    }
}

pub(crate) const XMP_INST_NO_DEFAULT_PAN: i32 = -1;
pub(crate) const XMP_INST_NNA_CUT: i32 = 0x00;
pub(crate) const XMP_INST_NNA_CONT: i32 = 0x01;
pub(crate) const XMP_INST_NNA_OFF: i32 = 0x02;
pub(crate) const XMP_INST_NNA_FADE: i32 = 0x03;
pub(crate) const XMP_INST_DCT_OFF: i32 = 0x00;
pub(crate) const XMP_INST_DCT_NOTE: i32 = 0x01;
pub(crate) const XMP_INST_DCT_SMP: i32 = 0x02;
pub(crate) const XMP_INST_DCT_INST: i32 = 0x03;
pub(crate) const XMP_INST_DCA_CUT: i32 = XMP_INST_NNA_CUT;
pub(crate) const XMP_INST_DCA_OFF: i32 = XMP_INST_NNA_OFF;
pub(crate) const XMP_INST_DCA_FADE: i32 = XMP_INST_NNA_FADE;

/// Translation of `struct xmp_subinstrument`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XmpSubinstrument {
    pub vol: i32, /* Default volume */
    pub gvl: i32, /* Global volume */
    pub pan: i32, /* Pan */
    pub xpo: i32, /* Transpose */
    pub fin: i32, /* Finetune */
    pub vwf: i32, /* Vibrato waveform */
    pub vde: i32, /* Vibrato depth */
    pub vra: i32, /* Vibrato rate */
    pub vsw: i32, /* Vibrato sweep */
    pub rvv: i32, /* Random volume/pan variation (IT) */
    pub sid: i32, /* Sample number */
    pub nna: i32, /* New note action */
    pub dct: i32, /* Duplicate check type */
    pub dca: i32, /* Duplicate check action */
    pub ifc: i32, /* Initial filter cutoff */
    pub ifr: i32, /* Initial filter resonance */
}

/// One entry of `struct xmp_instrument`'s `map`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XmpMap {
    pub ins: u8, /* Instrument number for each key */
    pub xpo: i8, /* Instrument transpose for each key */
}

/// Translation of `struct xmp_instrument`. `extra` is the format-specific
/// data upstream keeps behind a `void *` tagged with a magic number.
#[derive(Clone, Debug)]
pub(crate) struct XmpInstrument {
    pub name: [u8; 32],   /* Instrument name */
    pub vol: i32,         /* Instrument volume */
    pub nsm: i32,         /* Number of samples */
    pub rls: i32,         /* Release (fadeout) */
    pub aei: XmpEnvelope, /* Amplitude envelope info */
    pub pei: XmpEnvelope, /* Pan envelope info */
    pub fei: XmpEnvelope, /* Frequency envelope info */
    pub map: [XmpMap; XMP_MAX_KEYS],
    pub sub: Vec<XmpSubinstrument>,
    pub extra: extras::InstrumentExtras, /* Extra fields */
}

impl Default for XmpInstrument {
    fn default() -> Self {
        XmpInstrument {
            name: [0; 32],
            vol: 0,
            nsm: 0,
            rls: 0,
            aei: XmpEnvelope::default(),
            pei: XmpEnvelope::default(),
            fei: XmpEnvelope::default(),
            map: [XmpMap::default(); XMP_MAX_KEYS],
            sub: Vec::new(),
            extra: extras::InstrumentExtras::None,
        }
    }
}

pub(crate) const XMP_SAMPLE_16BIT: i32 = 1 << 0; /* 16bit sample */
pub(crate) const XMP_SAMPLE_LOOP: i32 = 1 << 1; /* Sample is looped */
pub(crate) const XMP_SAMPLE_LOOP_BIDIR: i32 = 1 << 2; /* Bidirectional sample loop */
pub(crate) const XMP_SAMPLE_LOOP_REVERSE: i32 = 1 << 3; /* Backwards sample loop */
pub(crate) const XMP_SAMPLE_LOOP_FULL: i32 = 1 << 4; /* Play full sample before looping */
pub(crate) const XMP_SAMPLE_SLOOP: i32 = 1 << 5; /* Sample has sustain loop */
pub(crate) const XMP_SAMPLE_SLOOP_BIDIR: i32 = 1 << 6; /* Bidirectional sustain loop */
pub(crate) const XMP_SAMPLE_STEREO: i32 = 1 << 7; /* Interlaced stereo sample */
pub(crate) const XMP_SAMPLE_SYNTH: i32 = 1 << 15; /* Data contains synth patch */

/// The guard bytes before a sample's data (upstream's `data` points past
/// them, and the mixer and the loaders read them at negative indices).
pub(crate) const SAMPLE_GUARD: usize = 4;

/// Translation of `struct xmp_sample`. `data`, when there is sample data,
/// holds [`SAMPLE_GUARD`] bytes before the samples upstream's `data`
/// points to.
#[derive(Clone, Debug, Default)]
pub(crate) struct XmpSample {
    pub name: [u8; 32],        /* Sample name */
    pub len: i32,              /* Sample length */
    pub lps: i32,              /* Loop start */
    pub lpe: i32,              /* Loop end */
    pub flg: i32,              /* Flags */
    pub data: Option<Vec<u8>>, /* Sample data */
}

/// Translation of `struct xmp_sequence`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XmpSequence {
    pub entry_point: i32,
    pub duration: i32,
}

pub(crate) const XMP_MARK_SKIP: u8 = 0xfe; /* S3M/IT-only: skip position */
pub(crate) const XMP_MARK_END: u8 = 0xff; /* S3M/IT-only: end position */

/// Translation of `struct xmp_module`. `xxp` and `xxt` are `None` where
/// upstream's pointers are `NULL`, and so is each pattern or track not
/// allocated.
#[derive(Clone, Debug)]
pub(crate) struct XmpModule {
    pub name: [u8; XMP_NAME_SIZE],  /* Module title */
    pub type_: [u8; XMP_NAME_SIZE], /* Module format */
    pub pat: i32,                   /* Number of patterns */
    pub trk: i32,                   /* Number of tracks */
    pub chn: i32,                   /* Tracks per pattern */
    pub ins: i32,                   /* Number of instruments */
    pub smp: i32,                   /* Number of samples */
    pub spd: i32,                   /* Initial speed */
    pub bpm: i32,                   /* Initial BPM */
    pub len: i32,                   /* Module length in patterns */
    pub rst: i32,                   /* Restart position */
    pub gvl: i32,                   /* Global volume */

    pub xxp: Option<Vec<Option<Box<XmpPattern>>>>, /* Patterns */
    pub xxt: Option<Vec<Option<Box<XmpTrack>>>>,   /* Tracks */
    pub xxi: Vec<XmpInstrument>,                   /* Instruments */
    pub xxs: Vec<XmpSample>,                       /* Samples */
    pub xxc: [XmpChannel; XMP_MAX_CHANNELS],       /* Channel info */

    pub xxo: [u8; XMP_MAX_MOD_LENGTH], /* Orders */
}

impl Default for XmpModule {
    fn default() -> Self {
        XmpModule {
            name: [0; XMP_NAME_SIZE],
            type_: [0; XMP_NAME_SIZE],
            pat: 0,
            trk: 0,
            chn: 0,
            ins: 0,
            smp: 0,
            spd: 0,
            bpm: 0,
            len: 0,
            rst: 0,
            gvl: 0,
            xxp: None,
            xxt: None,
            xxi: Vec::new(),
            xxs: Vec::new(),
            xxc: [XmpChannel::default(); XMP_MAX_CHANNELS],
            xxo: [0; XMP_MAX_MOD_LENGTH],
        }
    }
}

impl XmpModule {
    /// `mod->name` as a C string: one that fills its `XMP_NAME_SIZE` bytes
    /// runs on into `type`, which follows it in upstream's structure.
    pub fn name_cstr(&self) -> Vec<u8> {
        let mut s = cstr(&self.name).to_vec();
        if s.len() == self.name.len() {
            s.extend_from_slice(cstr(&self.type_));
        }
        s
    }

    /// `mod->xxp[i]`, which must exist.
    #[inline]
    pub fn pattern(&self, i: i32) -> &XmpPattern {
        self.xxp.as_ref().unwrap()[i as usize].as_deref().unwrap()
    }

    /// `mod->xxp[i]`, mutably.
    #[inline]
    pub fn pattern_mut(&mut self, i: i32) -> &mut XmpPattern {
        self.xxp.as_mut().unwrap()[i as usize]
            .as_deref_mut()
            .unwrap()
    }

    /// `mod->xxt[i]`, which must exist.
    #[inline]
    pub fn track(&self, i: i32) -> &XmpTrack {
        self.xxt.as_ref().unwrap()[i as usize].as_deref().unwrap()
    }

    /// `mod->xxt[i]`, mutably.
    #[inline]
    pub fn track_mut(&mut self, i: i32) -> &mut XmpTrack {
        self.xxt.as_mut().unwrap()[i as usize]
            .as_deref_mut()
            .unwrap()
    }

    /// `TRACK_NUM(a, c)`: `m->mod.xxp[a]->index[c]`.
    #[inline]
    pub fn track_num(&self, a: i32, c: i32) -> i32 {
        self.pattern(a).index[c as usize]
    }

    /// `EVENT(a, c, r)`: `m->mod.xxt[TRACK_NUM(a, c)]->event[r]`.
    #[inline]
    pub fn event(&self, a: i32, c: i32, r: i32) -> &XmpEvent {
        &self.track(self.track_num(a, c)).event[r as usize]
    }

    /// `EVENT(a, c, r)`, mutably.
    #[inline]
    pub fn event_mut(&mut self, a: i32, c: i32, r: i32) -> &mut XmpEvent {
        let t = self.track_num(a, c);
        &mut self.track_mut(t).event[r as usize]
    }

    /// `mod->xxo[i]`. Upstream reads `xxo[len]` in some places (the
    /// restart position check), which can be one past the array, into the
    /// module data that follows (a `NULL` pointer when loading from
    /// callbacks): 0 here.
    // FIXME (upstream): xxo[mod->rst] is read with rst == len == 256.
    #[inline]
    pub fn xxo(&self, i: i32) -> i32 {
        self.xxo.get(i as usize).map_or(0, |&v| v as i32)
    }
}

/// Translation of `struct xmp_test_info`.
#[derive(Clone, Debug)]
pub(crate) struct XmpTestInfo {
    pub name: [u8; XMP_NAME_SIZE],  /* Module title */
    pub type_: [u8; XMP_NAME_SIZE], /* Module format */
}

/// Translation of `struct xmp_module_info` (its pointers borrow the
/// context).
#[derive(Debug)]
pub(crate) struct XmpModuleInfo<'a> {
    pub md5: [u8; 16],               /* MD5 message digest */
    pub vol_base: i32,               /* Volume scale */
    pub mod_: &'a XmpModule,         /* Pointer to module data */
    pub comment: Option<&'a [u8]>,   /* Comment text, if any */
    pub num_sequences: i32,          /* Number of valid sequences */
    pub seq_data: &'a [XmpSequence], /* Pointer to sequence data */
}

/// Translation of `struct xmp_channel_info`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XmpChannelInfo {
    pub period: u32,     /* Sample period (* 4096) */
    pub position: u32,   /* Sample position */
    pub pitchbend: i16,  /* Linear bend from base note*/
    pub note: u8,        /* Current base note number */
    pub instrument: u8,  /* Current instrument number */
    pub sample: u8,      /* Current sample number */
    pub volume: u8,      /* Current volume */
    pub pan: u8,         /* Current stereo pan */
    pub reserved: u8,    /* Reserved */
    pub event: XmpEvent, /* Current track event */
}

/// Translation of `struct xmp_frame_info`. The sound buffer (`buffer`) is
/// the context's mixer buffer, [`control::frame_buffer`].
#[derive(Clone, Debug)]
pub(crate) struct XmpFrameInfo {
    pub pos: i32,           /* Current position */
    pub pattern: i32,       /* Current pattern */
    pub row: i32,           /* Current row in pattern */
    pub num_rows: i32,      /* Number of rows in current pattern */
    pub frame: i32,         /* Current frame */
    pub speed: i32,         /* Current replay speed */
    pub bpm: i32,           /* Current bpm */
    pub time: i32,          /* Current module time in ms */
    pub total_time: i32,    /* Estimated replay time in ms*/
    pub frame_time: i32,    /* Frame replay time in us */
    pub buffer_size: i32,   /* Used buffer size */
    pub total_size: i32,    /* Total buffer size */
    pub volume: i32,        /* Current master volume */
    pub loop_count: i32,    /* Loop counter */
    pub virt_channels: i32, /* Number of virtual channels */
    pub virt_used: i32,     /* Used virtual channels */
    pub sequence: i32,      /* Current sequence */

    pub channel_info: [XmpChannelInfo; XMP_MAX_CHANNELS], /* Current channel information */
}

impl Default for XmpFrameInfo {
    fn default() -> Self {
        XmpFrameInfo {
            pos: 0,
            pattern: 0,
            row: 0,
            num_rows: 0,
            frame: 0,
            speed: 0,
            bpm: 0,
            time: 0,
            total_time: 0,
            frame_time: 0,
            buffer_size: 0,
            total_size: 0,
            volume: 0,
            loop_count: 0,
            virt_channels: 0,
            virt_used: 0,
            sequence: 0,
            channel_info: [XmpChannelInfo::default(); XMP_MAX_CHANNELS],
        }
    }
}

/// Translation of `struct xmp_callbacks`: the stream a module is loaded
/// from (`read_func`, `seek_func`, `tell_func`; `close_func` is the
/// implementor's `Drop`, which SDL_mixer doesn't set).
pub(crate) trait XmpCallbacks {
    /// `read_func(dest, len, nmemb, priv)`: read `nmemb` items of `len`
    /// bytes into `dest`, returning the number of whole items read.
    fn read_func(&mut self, dest: &mut [u8], len: usize, nmemb: usize) -> usize;
    /// `seek_func(priv, offset, whence)` (`SEEK_SET`, `SEEK_CUR`,
    /// `SEEK_END`): 0, or -1 on failure.
    fn seek_func(&mut self, offset: i64, whence: i32) -> i32;
    /// `tell_func(priv)`: the position, or -1 on failure.
    fn tell_func(&mut self) -> i64;
}

/// `xmp_context`: the library's state for one module.
pub(crate) type XmpContext = Box<common::ContextData>;

pub(crate) use control::{
    xmp_create_context, xmp_free_context, xmp_seek_time, xmp_set_position, xmp_stop_module,
};
pub(crate) use load::{
    xmp_load_module_from_callbacks, xmp_release_module, xmp_test_module_from_callbacks,
};
pub(crate) use player::{
    xmp_end_player, xmp_get_frame_info, xmp_get_module_info, xmp_play_buffer, xmp_play_frame,
    xmp_start_player,
};

/// The bytes of a C string in a buffer (up to its NUL).
pub(crate) fn cstr(s: &[u8]) -> &[u8] {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    &s[..n]
}
