// Rust translation of src/loaders/med.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player (the header has no notice of its own; libxmp's:)
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The MED and OctaMED formats' structures and versions.
//!
//! The structures' pointers (offsets in the files) are read into the
//! loaders' locals, as upstream does, so they aren't fields here; nor are
//! the structures the loaders don't read.

pub(crate) const MMD_INST_TYPES: i32 = 9;

pub(crate) const MED_VER_210: i32 = 0x0210;
pub(crate) const MED_VER_300: i32 = 0x0300;
pub(crate) const MED_VER_320: i32 = 0x0320;
pub(crate) const MED_VER_OCTAMED_100: i32 = 0x1000;
pub(crate) const MED_VER_OCTAMED_200: i32 = 0x2000;
pub(crate) const MED_VER_OCTAMED_300: i32 = 0x3000;
pub(crate) const MED_VER_OCTAMED_400: i32 = 0x4000;
pub(crate) const MED_VER_OCTAMED_500: i32 = 0x5000;
pub(crate) const MED_VER_OCTAMED_502: i32 = 0x5002;
pub(crate) const MED_VER_OCTAMED_SS_1: i32 = 0x6000;
pub(crate) const MED_VER_OCTAMED_SS_2: i32 = 0x7000;

/* Structures as defined in the MED/OctaMED MMD0 and MMD1 file formats,
 * revision 1, described by Teijo Kinnunen in Apr 25 1992
 */

/// Translation of `struct MMD0sample`.
#[derive(Clone, Copy, Default)]
pub(crate) struct Mmd0Sample {
    pub rep: u16,       /* offs: 0(s) */
    pub replen: u16,    /* offs: 2(s) */
    pub midich: u8,     /* offs: 4(s) */
    pub midipreset: u8, /* offs: 5(s) */
    pub svol: u8,       /* offs: 6(s) */
    pub strans: i8,     /* offs: 7(s) */
}

/// Translation of `struct MMD0song`.
#[derive(Clone, Copy)]
pub(crate) struct Mmd0Song {
    pub sample: [Mmd0Sample; 63], /* 63 * 8 bytes = 504 bytes */
    pub numblocks: u16,           /* offs: 504 */
    pub songlen: u16,             /* offs: 506 */
    pub playseq: [u8; 256],       /* offs: 508 */
    pub deftempo: u16,            /* offs: 764 */
    pub playtransp: i8,           /* offs: 766 */
    pub flags: u8,                /* offs: 767 */
    pub flags2: u8,               /* offs: 768 */
    pub tempo2: u8,               /* offs: 769 */
    pub trkvol: [u8; 16],         /* offs: 770 */
    pub mastervol: u8,            /* offs: 786 */
    pub numsamples: u8,           /* offs: 787 */
} /* length = 788 bytes */

impl Default for Mmd0Song {
    fn default() -> Self {
        Mmd0Song {
            sample: [Mmd0Sample::default(); 63],
            numblocks: 0,
            songlen: 0,
            playseq: [0; 256],
            deftempo: 0,
            playtransp: 0,
            flags: 0,
            flags2: 0,
            tempo2: 0,
            trkvol: [0; 16],
            mastervol: 0,
            numsamples: 0,
        }
    }
}

pub(crate) const FLAG_FILTERON: u8 = 0x1; /* the hardware audio filter is on */
pub(crate) const FLAG_JUMPINGON: u8 = 0x2; /* mouse pointer jumping on */
pub(crate) const FLAG_JUMP8TH: u8 = 0x4; /* ump every 8th line (not in OctaMED Pro) */
pub(crate) const FLAG_INSTRSATT: u8 = 0x8; /* sng+samples indicator (not useful in MMDs) */
pub(crate) const FLAG_VOLHEX: u8 = 0x10; /* volumes are HEX */
pub(crate) const FLAG_STSLIDE: u8 = 0x20; /* use ST/NT/PT compatible sliding */
pub(crate) const FLAG_8CHANNEL: u8 = 0x40; /* this is OctaMED 5-8 channel song */
pub(crate) const FLAG_SLOWHQ: u8 = 0x80; /* HQ V2-4 compatibility mode */
pub(crate) const FLAG2_BMASK: u8 = 0x1F; /* (bits 0-4) BPM beat length (in lines) */
pub(crate) const FLAG2_BPM: u8 = 0x20; /* BPM mode on */
pub(crate) const FLAG2_MIX: u8 = 0x80; /* Module uses mixing */

/* This structure is exactly as long as the MMDsong structure. Common fields
 * are located at same offsets. You can also see, that there's a lot of room
 * for expansion in this structure.
 */
/// Translation of `struct MMD2song` (without its pointers and padding).
#[derive(Clone, Copy)]
pub(crate) struct Mmd2Song {
    pub sample: [Mmd0Sample; 63],
    pub numblocks: u16,
    pub songlen: u16, /* NOTE: number of sections in MMD2 */
    pub numtracks: u16, /* max. number of tracks in the song
                       * (also the number of entries in
                       * 'trackvols' table) */
    pub numpseqs: u16,     /* number of PlaySeqs in 'playseqtable' */
    pub flags3: u32,       /* see defs below */
    pub voladj: u16,       /* volume adjust (%), 0 means 100 */
    pub channels: u16,     /* mixing channels, 0 means 4 */
    pub mix_echotype: u8,  /* 0 = nothing, 1 = normal, 2 = cross */
    pub mix_echodepth: u8, /* 1 - 6, 0 = default */
    pub mix_echolen: u16,  /* echo length in milliseconds */
    pub mix_stereosep: i8, /* stereo separation */
    /* Fields below are MMD0/MMD1-compatible (except pad1[]) */
    pub deftempo: u16,
    pub playtransp: i8,
    pub flags: u8,
    pub flags2: u8,
    pub tempo2: u8,
    pub mastervol: u8,
    pub numsamples: u8,
} /* length = 788 bytes */

impl Default for Mmd2Song {
    fn default() -> Self {
        Mmd2Song {
            sample: [Mmd0Sample::default(); 63],
            numblocks: 0,
            songlen: 0,
            numtracks: 0,
            numpseqs: 0,
            flags3: 0,
            voladj: 0,
            channels: 0,
            mix_echotype: 0,
            mix_echodepth: 0,
            mix_echolen: 0,
            mix_stereosep: 0,
            deftempo: 0,
            playtransp: 0,
            flags: 0,
            flags2: 0,
            tempo2: 0,
            mastervol: 0,
            numsamples: 0,
        }
    }
}

pub(crate) const FLAG3_STEREO: u32 = 0x1; /* Mixing in stereo */
pub(crate) const FLAG3_FREEPAN: u32 = 0x2; /* Mixing flag: free pan */

/// Translation of `struct MMD0` (without its pointers).
#[derive(Clone, Copy, Default)]
pub(crate) struct Mmd0 {
    pub id: [u8; 4],
    pub modlen: u32,
    pub psecnum: u16, /* MMD2 only */
    pub pseq: u16,    /* MMD2 only */
    pub mmdflags: u8, /* MMD2 only */
    pub pstate: u16,  /* some data for the player routine */
    pub pblock: u16,
    pub pline: u16,
    pub pseqnum: u16,
    pub actplayline: i16,
    pub counter: u8,
    pub extra_songs: u8, /* number of songs - 1 */
} /* length = 52 bytes */

pub(crate) const MMD_LOADTOFASTMEM: u8 = 0x1;

/// Translation of `struct MMD1Block` (and `struct MMD0Block`, read into
/// it).
#[derive(Clone, Copy, Default)]
pub(crate) struct Mmd1Block {
    pub numtracks: u16,
    pub lines: u16,
}

/// Translation of `struct InstrHdr`.
#[derive(Clone, Copy, Default)]
pub(crate) struct InstrHdr {
    pub length: u32,
    pub type_: i16,
    /* Followed by actual data */
}

pub(crate) const S_16: i32 = 0x10; /* 16-bit sample */
pub(crate) const MD16: i32 = 0x18; /* 16-bit sample (Aura) */
pub(crate) const STEREO: i32 = 0x20; /* Stereo sample, not interleaved */

/// Translation of `struct SynthInstr`.
#[derive(Clone, Copy)]
pub(crate) struct SynthInstr {
    pub length: u32, /* length of this struct */
    pub type_: i16,  /* -1 or -2 (offs: 4) */
    pub defaultdecay: u8,
    pub rep: u16,
    pub replen: u16,
    pub voltbllen: u16,    /* offs: 14 */
    pub wftbllen: u16,     /* offs: 16 */
    pub volspeed: u8,      /* offs: 18 */
    pub wfspeed: u8,       /* offs: 19 */
    pub wforms: u16,       /* offs: 20 */
    pub voltbl: [u8; 128], /* offs: 22 */
    pub wftbl: [u8; 128],  /* offs: 150 */
    pub wf: [u32; 64],     /* offs: 278 */
}

impl Default for SynthInstr {
    fn default() -> Self {
        SynthInstr {
            length: 0,
            type_: 0,
            defaultdecay: 0,
            rep: 0,
            replen: 0,
            voltbllen: 0,
            wftbllen: 0,
            volspeed: 0,
            wfspeed: 0,
            wforms: 0,
            voltbl: [0; 128],
            wftbl: [0; 128],
            wf: [0; 64],
        }
    }
}

/* OctaMED SoundStudio 1 and prior use the InstrExt default_pitch field as a
 * default note value for the default note key 'F'. Pressing 'F' will insert a
 * note event with this note value.
 *
 * MED Soundstudio 2 in mix mode treats note 0x01 as a default note event,
 * which is emitted by the default note key instead of a regular note event.
 * It also makes this more complicated, despite not having changed the file
 * format: the user must enter a frequency in Hz instead of a note number,
 * where 8363 Hz corresponds to the event C-2. This frequency is converted to a
 * note number upon saving the module. Multi-octave instruments do not support
 * this feature as they are not supported by MED Soundstudio 2.
 *
 * This editor-only behavior would be irrelevant, except when default_pitch
 * is zero, the player uses the default frequency 22050 Hz instead. This
 * results in a note between E-3 and F-3. Since this feature is currently
 * implemented in the instrument map, use the mix mode note for F-3 instead.
 */
pub(crate) const MMD3_DEFAULT_NOTE: i32 = 53;

/// Translation of `struct InstrExt`.
#[derive(Clone, Copy, Default)]
pub(crate) struct InstrExt {
    pub hold: u8,
    pub decay: u8,
    pub suppress_midi_off: u8,
    pub finetune: i8,
    /* Below fields saved by >= V5 */
    pub default_pitch: u8,
    pub instr_flags: u8,
    pub long_midi_preset: u16,
    /* Below fields saved by >= V5.02 */
    pub output_device: u8,
    pub reserved: u8,
    /* Below fields saved by >= V7 */
    pub long_repeat: u32,
    pub long_replen: u32,
}

pub(crate) const SSFLG_LOOP: u8 = 0x01; /* Loop On/Off */
pub(crate) const SSFLG_EXTPSET: u8 = 0x02; /* Ext. Preset */
pub(crate) const SSFLG_DISABLED: u8 = 0x04; /* Disabled */
pub(crate) const SSFLG_PINGPONG: u8 = 0x08; /* Ping-pong looping */

/// Translation of `struct MMD0exp` (without its pointers).
#[derive(Clone, Copy, Default)]
pub(crate) struct Mmd0Exp {
    pub s_ext_entries: u16,    /* size of InstrExt structure array */
    pub s_ext_entrsz: u16,     /* size of each InstrExt structure */
    pub annolen: u32,          /* length of 'annotxt' */
    pub i_ext_entries: u16,    /* size of MMDInstrInfo struct array */
    pub i_ext_entrsz: u16,     /* size of each MMDInstrInfo struct */
    pub jumpmask: u32,         /* mouse pointer jump control */
    pub channelsplit: [u8; 4], /* channel splitting control */
    pub songnamelen: u32,      /* song name length */
}

/* (the MMDInfo, MMDARexx, MMDMIDICmd3x, MMDInstrInfo, NotationInfo,
 * MMDDumpData and MMDDump structures aren't read as such by libxmp) */
