/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * SoundFont file loading code borrowed from Smurf SoundFont Editor
 * Copyright (C) 1999-2001 Josh Green
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
// Modified 2026-10-17: translated into Rust from fluid_sffile.c and
// fluid_sffile.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).
// SDL_mixer's build has no libsndfile, so SoundFont 3 (Ogg Vorbis)
// samples are not supported (fluid_sffile_read_vorbis() fails, as
// upstream's without libsndfile), and fluid_is_soundfont() (not used by
// SDL_mixer) is left out. The `fluid_list_t`s are `Vec`s, the not yet
// filled placeholder entries `None`.

use crate::fluidsynth_h::*;
use crate::sfloader::sfont::{FluidFileCallbacks, SfFile};
use crate::synth::gen::*;
use crate::utils::sys::{
    flog, fluid_log, FLUID_DBG, FLUID_ERR, FLUID_PANIC, FLUID_WARN, SEEK_CUR, SEEK_END, SEEK_SET,
};

/*=================================sfload.c========================
Borrowed from Smurf SoundFont Editor by Josh Green
=================================================================*/

/// `FLUID_FOURCC()`
const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from_le_bytes([a, b, c, d])
}

/* FOURCC definitions */
const RIFF_FCC: u32 = fourcc(b'R', b'I', b'F', b'F');
const LIST_FCC: u32 = fourcc(b'L', b'I', b'S', b'T');
const SFBK_FCC: u32 = fourcc(b's', b'f', b'b', b'k');
const INFO_FCC: u32 = fourcc(b'I', b'N', b'F', b'O');
const SDTA_FCC: u32 = fourcc(b's', b'd', b't', b'a');
const PDTA_FCC: u32 = fourcc(b'p', b'd', b't', b'a'); /* info/sample/preset */

const IFIL_FCC: u32 = fourcc(b'i', b'f', b'i', b'l');
const ISNG_FCC: u32 = fourcc(b'i', b's', b'n', b'g');
const INAM_FCC: u32 = fourcc(b'I', b'N', b'A', b'M');
const IROM_FCC: u32 = fourcc(b'i', b'r', b'o', b'm'); /* info ids (1st byte of info strings) */
const IVER_FCC: u32 = fourcc(b'i', b'v', b'e', b'r');
const ICRD_FCC: u32 = fourcc(b'I', b'C', b'R', b'D');
const IENG_FCC: u32 = fourcc(b'I', b'E', b'N', b'G');
const IPRD_FCC: u32 = fourcc(b'I', b'P', b'R', b'D'); /* more info ids */
const ICOP_FCC: u32 = fourcc(b'I', b'C', b'O', b'P');
const ICMT_FCC: u32 = fourcc(b'I', b'C', b'M', b'T');
const ISFT_FCC: u32 = fourcc(b'I', b'S', b'F', b'T'); /* and yet more info ids */
const DMOD_FCC: u32 = fourcc(b'D', b'M', b'O', b'D'); // Default Modulators (https://github.com/spessasus/soundfont-proposals/blob/main/default_modulators.md)

const SNAM_FCC: u32 = fourcc(b's', b'n', b'a', b'm');
const SMPL_FCC: u32 = fourcc(b's', b'm', b'p', b'l'); /* sample ids */
const PHDR_FCC: u32 = fourcc(b'p', b'h', b'd', b'r');
const PBAG_FCC: u32 = fourcc(b'p', b'b', b'a', b'g');
const PMOD_FCC: u32 = fourcc(b'p', b'm', b'o', b'd');
const PGEN_FCC: u32 = fourcc(b'p', b'g', b'e', b'n'); /* preset ids */
const IHDR_FCC: u32 = fourcc(b'i', b'n', b's', b't');
const IBAG_FCC: u32 = fourcc(b'i', b'b', b'a', b'g');
const IMOD_FCC: u32 = fourcc(b'i', b'm', b'o', b'd');
const IGEN_FCC: u32 = fourcc(b'i', b'g', b'e', b'n'); /* instrument ids */
const SHDR_FCC: u32 = fourcc(b's', b'h', b'd', b'r'); /* sample info */
const SM24_FCC: u32 = fourcc(b's', b'm', b'2', b'4');

/*
 * This declares a uint32_t array containing the SF2 chunk identifiers.
 */
static IDLIST: [u32; 30] = [
    RIFF_FCC, LIST_FCC, SFBK_FCC, INFO_FCC, SDTA_FCC, PDTA_FCC, IFIL_FCC, ISNG_FCC, INAM_FCC,
    IROM_FCC, IVER_FCC, ICRD_FCC, IENG_FCC, IPRD_FCC, ICOP_FCC, ICMT_FCC, ISFT_FCC, DMOD_FCC,
    SNAM_FCC, SMPL_FCC, PHDR_FCC, PBAG_FCC, PMOD_FCC, PGEN_FCC, IHDR_FCC, IBAG_FCC, IMOD_FCC,
    IGEN_FCC, SHDR_FCC, SM24_FCC,
];

/* Set when the FCC code is unknown */
const UNKN_ID: usize = IDLIST.len();

static INVALID_INST_GEN: [u16; 8] = [
    GEN_UNUSED1 as u16,
    GEN_UNUSED2 as u16,
    GEN_UNUSED3 as u16,
    GEN_UNUSED4 as u16,
    GEN_RESERVED1 as u16,
    GEN_RESERVED2 as u16,
    GEN_RESERVED3 as u16,
    GEN_INSTRUMENT as u16,
];

static INVALID_PRESET_GEN: [u16; 14] = [
    GEN_STARTADDROFS as u16,
    GEN_ENDADDROFS as u16,
    GEN_STARTLOOPADDROFS as u16,
    GEN_ENDLOOPADDROFS as u16,
    GEN_STARTADDRCOARSEOFS as u16,
    GEN_ENDADDRCOARSEOFS as u16,
    GEN_STARTLOOPADDRCOARSEOFS as u16,
    GEN_KEYNUM as u16,
    GEN_VELOCITY as u16,
    GEN_ENDLOOPADDRCOARSEOFS as u16,
    GEN_SAMPLEMODE as u16,
    GEN_EXCLUSIVECLASS as u16,
    GEN_OVERRIDEROOTKEY as u16,
    GEN_SAMPLEID as u16,
];

/* sfont file chunk sizes */
const SF_PHDR_SIZE: u32 = 38;
const SF_BAG_SIZE: i32 = 4;
const SF_MOD_SIZE: i32 = 10;
const SF_GEN_SIZE: i32 = 4;
const SF_IHDR_SIZE: u32 = 22;
const SF_SHDR_SIZE: u32 = 46;

/* Sound Font structure defines */

/// Translation of `SFVersion`: version structure.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SfVersion {
    pub major: u16,
    pub minor: u16,
}

/// Translation of `SFMod`: Modulator structure.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SfMod {
    pub src: u16,    /* source modulator */
    pub dest: u16,   /* destination generator */
    pub amount: i16, /* signed, degree of modulation */
    pub amtsrc: u16, /* second source controls amnt of first */
    pub trans: u16,  /* transform applied to source */
}

/// Translation of `SFGenAmount`: generator amount structure (a union in
/// C: the 16 bit value as read, little-endian).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SfGenAmount(pub u16);

impl SfGenAmount {
    /// The `sword` member: signed 16 bit value
    pub(crate) fn sword(self) -> i16 {
        self.0 as i16
    }
    /// The `uword` member: unsigned 16 bit value
    pub(crate) fn uword(self) -> u16 {
        self.0
    }
    /// The `range.lo` member: low value for ranges
    pub(crate) fn lo(self) -> u8 {
        self.0 as u8
    }
    /// The `range.hi` member: high value for ranges
    pub(crate) fn hi(self) -> u8 {
        (self.0 >> 8) as u8
    }
}

/// Translation of `SFGen`: Generator structure.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SfGen {
    pub id: u16,             /* generator ID */
    pub amount: SfGenAmount, /* generator value */
}

/// Translation of `SFZone`: Sample/instrument zone structure.
#[derive(Clone, Debug, Default)]
pub(crate) struct SfZone {
    pub gen: Vec<Option<SfGen>>,  /* list of generators */
    pub mod_: Vec<Option<SfMod>>, /* list of modulators */
}

/// Translation of `SFSample`: Sample structure.
#[derive(Clone, Debug, Default)]
pub(crate) struct SfSample {
    pub name: String, /* Name of sample */
    pub idx: i32,     /* Index of this instrument in the Soundfont */
    pub start: u32,   /* Offset in sample area to start of sample */
    pub end: u32,     /* Offset from start to end of sample,
                      this is the last point of the
                      sample, the SF spec has this as the
                      1st point after, corrected on
                      load/save */
    pub loopstart: u32, /* Offset from start to start of loop */
    pub loopend: u32,   /* Offset from start to end of loop,
                        marks the first point after loop,
                        whose sample value is ideally
                        equivalent to loopstart */
    pub samplerate: u32,             /* Sample rate recorded at */
    pub origpitch: u8,               /* root midi key number */
    pub pitchadj: i8,                /* pitch correction in cents */
    pub sampletype: u16,             /* 1 mono,2 right,4 left,linked 8,0x8000=ROM */
    pub fluid_sample: Option<usize>, /* Imported sample (fixed up in fluid_defsfont_load): its index in the defsfont's samples */
}

/// Translation of `SFInst`: Instrument structure.
#[derive(Clone, Debug, Default)]
pub(crate) struct SfInst {
    pub name: String,              /* Name of instrument */
    pub idx: i32,                  /* Index of this instrument in the Soundfont */
    pub zone: Vec<Option<SfZone>>, /* list of instrument zones */
}

/// Translation of `SFPreset`: Preset structure.
#[derive(Clone, Debug, Default)]
pub(crate) struct SfPreset {
    pub name: String,              /* preset name */
    pub prenum: u16,               /* preset number */
    pub bank: u16,                 /* bank number */
    pub zone: Vec<Option<SfZone>>, /* list of preset zones */
}

/// Translation of `SFData`: Sound font data structure.
///
/// NOTE: sffd is also used to determine if sound font is new (NULL)
pub(crate) struct SfData {
    pub version: SfVersion, /* sound font version */
    pub romver: SfVersion,  /* ROM version */

    pub filesize: u32,

    pub samplepos: u32,  /* position within sffd of the sample chunk */
    pub samplesize: u32, /* length within sffd of the sample chunk */

    pub sample24pos: u32, /* position within sffd of the sm24 chunk, set to zero if no 24 bit
                          sample support */
    pub sample24size: u32, /* length within sffd of the sm24 chunk */

    pub hydrapos: u32,
    pub hydrasize: u32,

    pub fname: String,                 /* file name */
    pub sffd: Option<Box<dyn SfFile>>, /* loaded sfont file descriptor */
    #[allow(dead_code)]
    pub fcbs: FluidFileCallbacks, /* file callbacks used to read this file */

    pub info: Vec<Vec<u8>>, /* linked list of info strings (1st byte is ID) */
    pub preset: Vec<SfPreset>, /* linked list of preset info */
    pub inst: Vec<SfInst>,  /* linked list of instrument info */
    pub sample: Vec<SfSample>, /* linked list of sample info */
    pub default_mod_list: Vec<SfMod>, /* default modulator list */
}

/*-----------------------------------sffile.h----------------------------*/
/*
   File structures and routines (used to be in sffile.h)
*/

/// Translation of `SFChunk`: RIFF file chunk structure.
#[derive(Clone, Copy, Debug, Default)]
struct SfChunk {
    id: u32,   /* chunk id */
    size: u32, /* size of the following chunk */
}

impl SfData {
    fn fd(&mut self) -> &mut dyn SfFile {
        // (sffd is set from fluid_sffile_open() until fluid_sffile_close())
        self.sffd.as_deref_mut().expect("SoundFont file open")
    }

    /// `sf->fcbs->fread()`
    fn fread(&mut self, buf: &mut [u8]) -> bool {
        self.fd().read(buf) != FLUID_FAILED
    }

    /// `sf->fcbs->fseek()`
    fn fseek(&mut self, offset: i64, whence: i32) -> bool {
        self.fd().seek(offset, whence) != FLUID_FAILED
    }

    /// `sf->fcbs->ftell()`
    fn ftell(&mut self) -> i64 {
        self.fd().tell()
    }

    /// `READCHUNK()`
    fn readchunk(&mut self) -> Option<SfChunk> {
        let mut id = [0u8; 4];
        if !self.fread(&mut id) {
            return None;
        }
        let mut size = [0u8; 4];
        if !self.fread(&mut size) {
            return None;
        }
        Some(SfChunk {
            id: u32::from_le_bytes(id),
            size: u32::from_le_bytes(size),
        })
    }

    /// `READD()`
    fn readd(&mut self) -> Option<u32> {
        let mut temp = [0u8; 4];
        if !self.fread(&mut temp) {
            return None;
        }
        Some(u32::from_le_bytes(temp))
    }

    /// `READW()`
    fn readw(&mut self) -> Option<u16> {
        let mut temp = [0u8; 2];
        if !self.fread(&mut temp) {
            return None;
        }
        Some(u16::from_le_bytes(temp))
    }

    /// `READID()`
    fn readid(&mut self) -> Option<u32> {
        let mut id = [0u8; 4];
        if !self.fread(&mut id) {
            return None;
        }
        Some(u32::from_le_bytes(id))
    }

    /// `READSTR()`: 20 bytes, NUL terminated at the 21st.
    fn readstr(&mut self) -> Option<String> {
        let mut buf = [0u8; 20];
        if !self.fread(&mut buf) {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(20);
        Some(String::from_utf8_lossy(&buf[..len]).into_owned())
    }

    /// `READB()`
    fn readb(&mut self) -> Option<u8> {
        let mut b = [0u8; 1];
        if !self.fread(&mut b) {
            return None;
        }
        Some(b[0])
    }

    /// `FSKIP()`
    fn fskip(&mut self, size: i64) -> bool {
        self.fseek(size, SEEK_CUR)
    }

    /// `FSKIPW()`
    fn fskipw(&mut self) -> bool {
        self.fseek(2, SEEK_CUR)
    }
}

/// `?` for the READ*() and FSKIP() macros, which `return FALSE`.
macro_rules! try_read {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return false,
        }
    };
}

macro_rules! try_skip {
    ($e:expr) => {
        if !$e {
            return false;
        }
    };
}

/// Translation of `fluid_sffile_open()`: open a SoundFont file and parse
/// it's contents into a SFData structure.
///
/// @param fname filename
/// @param fcbs file callback structure
/// @return the partially parsed SoundFont as SFData structure or NULL on error
pub(crate) fn fluid_sffile_open(fname: &str, fcbs: &FluidFileCallbacks) -> Option<SfData> {
    let mut sf = SfData {
        version: SfVersion::default(),
        romver: SfVersion::default(),
        filesize: 0,
        samplepos: 0,
        samplesize: 0,
        sample24pos: 0,
        sample24size: 0,
        hydrapos: 0,
        hydrasize: 0,
        fname: String::new(),
        sffd: None,
        fcbs: fcbs.clone(),
        info: Vec::new(),
        preset: Vec::new(),
        inst: Vec::new(),
        sample: Vec::new(),
        default_mod_list: Vec::new(),
    };

    sf.sffd = (fcbs.fopen)(fname);
    if sf.sffd.is_none() {
        flog!(FLUID_ERR, "Unable to open file '{}'", fname);
        fluid_sffile_close(sf);
        return None;
    }

    sf.fname = fname.to_string();

    /* get size of file by seeking to end */
    if !sf.fseek(0, SEEK_END) {
        fluid_log(FLUID_ERR, "Seek to end of file failed");
        fluid_sffile_close(sf);
        return None;
    }

    let fsize = sf.ftell();
    if fsize == FLUID_FAILED as i64 {
        fluid_log(FLUID_ERR, "Get end of file position failed");
        fluid_sffile_close(sf);
        return None;
    }

    sf.filesize = fsize as u32;

    if !sf.fseek(0, SEEK_SET) {
        fluid_log(FLUID_ERR, "Rewind to start of file failed");
        fluid_sffile_close(sf);
        return None;
    }

    if !load_header(&mut sf) {
        fluid_sffile_close(sf);
        return None;
    }

    Some(sf)
}

/// Translation of `fluid_sffile_parse_presets()`: parse all preset
/// information from the soundfont.
///
/// @return FLUID_OK on success, otherwise FLUID_FAILED
pub(crate) fn fluid_sffile_parse_presets(sf: &mut SfData) -> i32 {
    if !load_body(sf) {
        return FLUID_FAILED;
    }

    FLUID_OK
}

/// Translation of `fluid_sffile_read_sample_data()`: load sample data from
/// the soundfont file.
///
/// This function will always return the sample data in WAV format. If the sample_type specifies an
/// Ogg Vorbis compressed sample, it will be decompressed automatically before returning.
///
/// @param sf SFData instance
/// @param sample_start index of first sample point in Soundfont sample chunk
/// @param sample_end index of last sample point in Soundfont sample chunk
/// @param sample_type type of the sample in Soundfont
/// @return The 16 bit sample data and the 24-bit sample data if present
/// in the file (the number of sample words is the former's length), or
/// None on failure
pub(crate) fn fluid_sffile_read_sample_data(
    sf: &mut SfData,
    sample_start: u32,
    sample_end: u32,
    sample_type: i32,
) -> Option<(Vec<i16>, Option<Vec<u8>>)> {
    if sample_type & FLUID_SAMPLETYPE_OGG_VORBIS != 0 {
        fluid_sffile_read_vorbis(sf, sample_start, sample_end)
    } else {
        fluid_sffile_read_wav(sf, sample_start, sample_end)
    }
}

/// Translation of `fluid_sffile_close()`: close a SoundFont file and free
/// the SFData structure.
pub(crate) fn fluid_sffile_close(mut sf: SfData) {
    if let Some(sffd) = sf.sffd.take() {
        sffd.close();
    }
}

/*
 * Private functions
 */

/// Translation of `chunkid()`: sound font file load functions.
fn chunkid(id: u32) -> usize {
    /* Return chunk id or UNKN_ID if not found */
    IDLIST.iter().position(|&i| i == id).unwrap_or(UNKN_ID)
}

/// The four characters of a FOURCC, for `%.4s` (stopping at a NUL, as
/// `%.4s` does).
fn fcc_str(id: u32) -> String {
    let bytes = id.to_le_bytes();
    let len = bytes.iter().position(|&c| c == 0).unwrap_or(4);
    String::from_utf8_lossy(&bytes[..len]).into_owned()
}

/// Translation of `load_header()`.
fn load_header(sf: &mut SfData) -> bool {
    let mut chunk = try_read!(sf.readchunk()); /* load RIFF chunk */

    if chunk.id != RIFF_FCC {
        /* error if not RIFF */
        fluid_log(FLUID_ERR, "Not a RIFF file");
        return false;
    }

    chunk.id = try_read!(sf.readid()); /* load file ID */

    if chunk.id != SFBK_FCC {
        /* error if not SFBK_ID */
        fluid_log(FLUID_ERR, "Not a SoundFont file");
        return false;
    }

    if chunk.size != sf.filesize.wrapping_sub(8) {
        fluid_log(FLUID_ERR, "SoundFont file size mismatch");
        return false;
    }

    /* Process INFO block */
    if !read_listchunk(sf, &mut chunk) {
        return false;
    }

    if chunk.id != INFO_FCC {
        fluid_log(FLUID_ERR, "Invalid ID found when expecting INFO chunk");
        return false;
    }

    if !process_info(sf, chunk.size as i32) {
        return false;
    }

    /* Process sample chunk */
    if !read_listchunk(sf, &mut chunk) {
        return false;
    }

    if chunk.id != SDTA_FCC {
        fluid_log(FLUID_ERR, "Invalid ID found when expecting SAMPLE chunk");
        return false;
    }

    if !process_sdta(sf, chunk.size) {
        return false;
    }

    /* process HYDRA chunk */
    if !read_listchunk(sf, &mut chunk) {
        return false;
    }

    if chunk.id != PDTA_FCC {
        fluid_log(FLUID_ERR, "Invalid ID found when expecting HYDRA chunk");
        return false;
    }

    sf.hydrapos = sf.ftell() as u32;
    sf.hydrasize = chunk.size;

    true
}

/// Translation of `load_body()`.
fn load_body(sf: &mut SfData) -> bool {
    let hydrapos = sf.hydrapos;
    if !sf.fseek(hydrapos as i64, SEEK_SET) {
        fluid_log(FLUID_ERR, "Failed to seek to HYDRA position");
        return false;
    }

    let hydrasize = sf.hydrasize;
    if !process_pdta(sf, hydrasize as i32) {
        return false;
    }

    /* sort preset list by bank, preset # */
    let presets = std::mem::take(&mut sf.preset);
    sf.preset = fluid_list_sort(presets, preset_compare_func);

    true
}

/// Translation of `fluid_list_sort()` (fluid_list.c) with
/// `fluid_list_sort_merge()`: the linked list merge sort, which takes the
/// second element first among equal ones.
fn fluid_list_sort<T>(list: Vec<T>, compare_func: fn(&T, &T) -> i32) -> Vec<T> {
    if list.len() < 2 {
        return list;
    }

    // (the first half has floor(n/2) elements, as the two-pointer walk
    // upstream gives)
    let mut l1 = list;
    let l2 = l1.split_off(l1.len() / 2);
    let mut l1 = fluid_list_sort(l1, compare_func).into_iter().peekable();
    let mut l2 = fluid_list_sort(l2, compare_func).into_iter().peekable();

    let mut merged = Vec::with_capacity(l1.len() + l2.len());
    while let (Some(a), Some(b)) = (l1.peek(), l2.peek()) {
        if compare_func(a, b) < 0 {
            merged.extend(l1.next());
        } else {
            merged.extend(l2.next());
        }
    }
    merged.extend(l1);
    merged.extend(l2);
    merged
}

/// Translation of `read_listchunk()`.
fn read_listchunk(sf: &mut SfData, chunk: &mut SfChunk) -> bool {
    *chunk = try_read!(sf.readchunk()); /* read list chunk */

    if chunk.id != LIST_FCC {
        /* error if ! list chunk */
        let p = chunk.id.to_le_bytes();
        flog!(
            FLUID_ERR,
            "Invalid chunk id '0x{:X} 0x{:X} 0x{:X} 0x{:X}' ({} bytes) in level 0 parse",
            p[0] as i32,
            p[1] as i32,
            p[2] as i32,
            p[3] as i32,
            chunk.size as i32
        );
        return false;
    }

    chunk.id = try_read!(sf.readid()); /* read id string */
    chunk.size = chunk.size.wrapping_sub(4);
    true
}

/// Translation of `process_info()`.
fn process_info(sf: &mut SfData, mut size: i32) -> bool {
    sf.default_mod_list.clear();

    while size > 0 {
        let chunk = try_read!(sf.readchunk());
        size -= 8;
        // read the fourcc for debugging
        let p = chunk.id.to_le_bytes();

        if chunk.id == IFIL_FCC {
            /* sound font version chunk? */
            if chunk.size != 4 {
                fluid_log(FLUID_ERR, "Sound font version info chunk has invalid size");
                return false;
            }

            sf.version.major = try_read!(sf.readw());
            sf.version.minor = try_read!(sf.readw());
            flog!(
                FLUID_DBG,
                "SF Version: {}.{}",
                sf.version.major,
                sf.version.minor
            );

            if sf.version.major < 2 {
                flog!(
                    FLUID_ERR,
                    "Sound font version is {}.{} which is not supported, convert to version 2.0x",
                    sf.version.major,
                    sf.version.minor
                );
                return false;
            }

            if sf.version.major == 3 {
                /* !LIBSNDFILE_SUPPORT */
                flog!(
                    FLUID_WARN,
                    "Sound font version is {}.{} but fluidsynth was compiled without support for (v3.x)",
                    sf.version.major,
                    sf.version.minor
                );
                return false;
            } else if sf.version.major > 2 {
                flog!(
                    FLUID_WARN,
                    "Sound font version is {}.{} which is newer than what this version of fluidsynth was designed for (v2.0x)",
                    sf.version.major,
                    sf.version.minor
                );
                return false;
            }
        } else if chunk.id == IVER_FCC {
            /* ROM version chunk? */
            if chunk.size != 4 {
                fluid_log(FLUID_ERR, "ROM version info chunk has invalid size");
                return false;
            }

            sf.romver.major = try_read!(sf.readw());
            sf.romver.minor = try_read!(sf.readw());
            flog!(
                FLUID_DBG,
                "ROM Version: {}.{}",
                sf.version.major,
                sf.version.minor
            );
        } else if chunk.id == DMOD_FCC {
            /* Default modulators chunk
             * https://github.com/spessasus/soundfont-proposals/blob/main/default_modulators.md
             */
            if chunk.size < SF_MOD_SIZE as u32 || chunk.size % SF_MOD_SIZE as u32 != 0 || size == 0
            {
                flog!(
                    FLUID_ERR,
                    "DMOD chunk has invalid size ({} bytes)",
                    chunk.size as i32
                );
                return false;
            }

            // read the modulators sequentially
            let mut count = chunk.size / SF_MOD_SIZE as u32 - 1; // minus the terminal record
            flog!(
                FLUID_DBG,
                "Detected the DMOD chunk with {} modulators",
                count as i32
            );
            while count > 0 {
                let dmod = SfMod {
                    src: try_read!(sf.readw()),
                    dest: try_read!(sf.readw()),
                    amount: try_read!(sf.readw()) as i16,
                    amtsrc: try_read!(sf.readw()),
                    trans: try_read!(sf.readw()),
                };
                // (fluid_list_prepend())
                sf.default_mod_list.insert(0, dmod);
                count -= 1;
            }

            // terminal record
            try_skip!(sf.fskip(SF_MOD_SIZE as i64));
        } else {
            let is_info_chunk_256 = matches!(
                chunk.id,
                ISNG_FCC
                    | INAM_FCC
                    | IROM_FCC
                    | ICRD_FCC
                    | IENG_FCC
                    | IPRD_FCC
                    | ICOP_FCC
                    | ISFT_FCC
            );

            if chunk.size % 2 != 0 {
                flog!(
                    FLUID_ERR,
                    "INFO sub chunk {} has odd size of {} bytes, in violation of RIFF spec. Rejecting file as structurally defective.",
                    fcc_str(chunk.id),
                    chunk.size as i32
                );
                return false;
            }

            if is_info_chunk_256 || chunk.id == ICMT_FCC {
                if (is_info_chunk_256 && chunk.size > 256)
                    || (chunk.id == ICMT_FCC && chunk.size > 65536)
                {
                    flog!(
                        FLUID_WARN,
                        "Well known INFO sub chunk {} has invalid chunk size of {} bytes, discarding chunk.",
                        fcc_str(chunk.id),
                        chunk.size as i32
                    );

                    if !sf.fseek(chunk.size as i64, SEEK_CUR) {
                        return false;
                    }
                } else {
                    /* alloc for chunk fcc and da chunk */
                    let mut item = vec![0u8; chunk.size as usize + 4 + 1];

                    /* save chunk fcc and update pointer to data value */
                    item[..4].copy_from_slice(&chunk.id.to_le_bytes());

                    let ok = sf.fread(&mut item[4..4 + chunk.size as usize]);

                    /* attach to INFO list, fluid_sffile_close will cleanup if FAIL occurs */
                    sf.info.push(item);

                    if !ok {
                        return false;
                    }

                    /* force terminate info item */
                    // (already zero)
                }
            } else {
                /* SF 2.04 specification, section 10.2:
                 * 10.2 Unknown chunks
                 * within the INFO-list chunk should simply be ignored.
                 * Other unknown chunks or sub-chunks are illegal and should be
                 * treated as structural errors.*/
                flog!(
                    FLUID_WARN,
                    "Ignoring {} chunk ID '{}{}{}{}' in INFO chunk",
                    if chunkid(chunk.id) == UNKN_ID {
                        "unknown"
                    } else {
                        "unexpected"
                    },
                    p[0] as char,
                    p[1] as char,
                    p[2] as char,
                    p[3] as char
                );

                if !sf.fseek(chunk.size as i64, SEEK_CUR) {
                    return false;
                }
            }
            flog!(
                FLUID_DBG,
                "INFO chunk {}{}{}{} ({} bytes)",
                p[0] as char,
                p[1] as char,
                p[2] as char,
                p[3] as char,
                chunk.size as i32
            );
        }

        size = size.wrapping_sub(chunk.size as i32);
    }

    if size < 0 {
        fluid_log(FLUID_ERR, "INFO chunk size mismatch");
        return false;
    }

    true
}

/// Translation of `process_sdta()`.
fn process_sdta(sf: &mut SfData, mut size: u32) -> bool {
    if size == 0 {
        return true; /* no sample data? */
    }

    /* read sub chunk */
    let mut chunk = try_read!(sf.readchunk());
    size = size.wrapping_sub(8);

    if chunk.id != SMPL_FCC {
        fluid_log(FLUID_ERR, "Expected SMPL chunk found invalid id instead");
        return false;
    }

    /* SDTA chunk may also contain sm24 chunk for 24 bit samples
     * (not yet supported), only an error if SMPL chunk size is
     * greater than SDTA. */
    if chunk.size > size {
        fluid_log(FLUID_ERR, "SDTA chunk size mismatch");
        return false;
    }

    /* sample data follows */
    sf.samplepos = sf.ftell() as u32;

    /* used to check validity of sample headers */
    sf.samplesize = chunk.size;

    try_skip!(sf.fskip(chunk.size as i64));
    size -= chunk.size;

    'ret: {
        if sf.version.major >= 2 && sf.version.minor >= 4 {
            /* any chance to find another chunk here? */
            if size > 8 {
                /* read sub chunk */
                chunk = try_read!(sf.readchunk());
                size -= 8;

                if chunk.id == SM24_FCC {
                    fluid_log(FLUID_DBG, "Found SM24 chunk");

                    if chunk.size > size {
                        fluid_log(FLUID_WARN, "SM24 exceeds SDTA chunk, ignoring SM24");
                        break 'ret; // no error
                    }

                    let mut sdtahalfsize = (sf.samplesize / 2) as i32;
                    /* + 1 byte in the case that half the size of smpl chunk is an odd value */
                    sdtahalfsize += sdtahalfsize % 2;
                    let sm24size = chunk.size as i32;

                    if sdtahalfsize != sm24size {
                        flog!(
                            FLUID_WARN,
                            "SM24 not equal to half the size of SMPL chunk (0x{:X} != 0x{:X}), ignoring SM24",
                            sm24size,
                            sdtahalfsize
                        );
                        break 'ret; // no error
                    }

                    /* sample data24 follows */
                    sf.sample24pos = sf.ftell() as u32;
                    sf.sample24size = sm24size as u32;
                }
            }
        }
    }

    try_skip!(sf.fskip(size as i64));

    true
}

/// Translation of `pdtahelper()`.
fn pdtahelper(
    sf: &mut SfData,
    expid: u32,
    reclen: u32,
    chunk: &mut SfChunk,
    size: &mut i32,
) -> bool {
    *chunk = try_read!(sf.readchunk());
    *size = size.wrapping_sub(8);

    if chunk.id != expid {
        flog!(
            FLUID_ERR,
            "Expected PDTA sub-chunk '{}' found invalid id instead",
            fcc_str(expid)
        );
        return false;
    }

    if !chunk.size.is_multiple_of(reclen) {
        /* valid chunk size? */
        flog!(
            FLUID_ERR,
            "'{}' chunk size is not a multiple of {} bytes",
            fcc_str(expid),
            reclen
        );
        return false;
    }

    *size = size.wrapping_sub(chunk.size as i32);
    if *size < 0 {
        flog!(
            FLUID_ERR,
            "'{}' chunk size exceeds remaining PDTA chunk size",
            fcc_str(expid)
        );
        return false;
    }

    true
}

/// Translation of `process_pdta()`.
fn process_pdta(sf: &mut SfData, mut size: i32) -> bool {
    let mut chunk = SfChunk::default();

    if !pdtahelper(sf, PHDR_FCC, SF_PHDR_SIZE, &mut chunk, &mut size) {
        return false;
    }

    if !load_phdr(sf, chunk.size) {
        return false;
    }

    if !pdtahelper(sf, PBAG_FCC, SF_BAG_SIZE as u32, &mut chunk, &mut size) {
        return false;
    }

    if !load_pbag(sf, chunk.size as i32) {
        return false;
    }

    if !pdtahelper(sf, PMOD_FCC, SF_MOD_SIZE as u32, &mut chunk, &mut size) {
        return false;
    }

    if !load_pmod(sf, chunk.size as i32) {
        return false;
    }

    if !pdtahelper(sf, PGEN_FCC, SF_GEN_SIZE as u32, &mut chunk, &mut size) {
        return false;
    }

    if !load_pgen(sf, chunk.size as i32) {
        return false;
    }

    if !pdtahelper(sf, IHDR_FCC, SF_IHDR_SIZE, &mut chunk, &mut size) {
        return false;
    }

    if !load_ihdr(sf, chunk.size) {
        return false;
    }

    if !pdtahelper(sf, IBAG_FCC, SF_BAG_SIZE as u32, &mut chunk, &mut size) {
        return false;
    }

    if !load_ibag(sf, chunk.size as i32) {
        return false;
    }

    if !pdtahelper(sf, IMOD_FCC, SF_MOD_SIZE as u32, &mut chunk, &mut size) {
        return false;
    }

    if !load_imod(sf, chunk.size as i32) {
        return false;
    }

    if !pdtahelper(sf, IGEN_FCC, SF_GEN_SIZE as u32, &mut chunk, &mut size) {
        return false;
    }

    if !load_igen(sf, chunk.size as i32) {
        return false;
    }

    if !pdtahelper(sf, SHDR_FCC, SF_SHDR_SIZE, &mut chunk, &mut size) {
        return false;
    }

    if !load_shdr(sf, chunk.size) {
        return false;
    }

    true
}

/// `fluid_list_prepend(list, NULL)`, `count` times: placeholders, filled
/// in later (all NULL, so where they go makes no difference). The
/// allocation is fallible, as the counts come from the file.
fn add_placeholders<T>(list: &mut Vec<Option<T>>, count: usize) -> bool {
    if list.try_reserve(count).is_err() {
        fluid_log(FLUID_PANIC, "Out of memory");
        return false;
    }
    list.extend((0..count).map(|_| None));
    true
}

/// Translation of `load_phdr()`: preset header loader.
fn load_phdr(sf: &mut SfData, size: u32) -> bool {
    let mut prev_preset: Option<usize> = None;
    let mut prev_pbag_idx: u16 = 0;

    if !size.is_multiple_of(SF_PHDR_SIZE) || size == 0 {
        fluid_log(FLUID_ERR, "Preset header chunk size is invalid");
        return false;
    }

    let mut i = size / SF_PHDR_SIZE - 1;

    if i == 0 {
        /* at least one preset + term record */
        fluid_log(FLUID_WARN, "File contains no presets");
        try_skip!(sf.fskip(SF_PHDR_SIZE as i64));
        return true;
    }

    while i > 0 {
        /* load all preset headers */
        sf.preset.push(SfPreset::default());
        let cur = sf.preset.len() - 1;
        sf.preset[cur].name = try_read!(sf.readstr()); /* possible read failure ^ */
        sf.preset[cur].prenum = try_read!(sf.readw());
        sf.preset[cur].bank = try_read!(sf.readw());
        let pbag_idx = try_read!(sf.readw());
        try_skip!(sf.fskip(4)); /* library ignored */
        try_skip!(sf.fskip(4)); /* genre ignored */
        try_skip!(sf.fskip(4)); /* morphology ignored */

        if let Some(prev) = prev_preset {
            /* not first preset? */
            if pbag_idx < prev_pbag_idx {
                fluid_log(FLUID_ERR, "Preset header indices not monotonic");
                return false;
            }

            let i2 = (pbag_idx - prev_pbag_idx) as usize;

            if !add_placeholders(&mut sf.preset[prev].zone, i2) {
                return false;
            }
        } else if pbag_idx > 0 {
            /* 1st preset, warn if ofs >0 */
            flog!(
                FLUID_WARN,
                "{} preset zones not referenced, discarding",
                pbag_idx
            );
        }

        prev_preset = Some(cur); /* update preset ptr */
        prev_pbag_idx = pbag_idx;
        i -= 1;
    }

    try_skip!(sf.fskip(24));
    let pbag_idx = try_read!(sf.readw()); /* Read terminal generator index */
    try_skip!(sf.fskip(12));

    if pbag_idx < prev_pbag_idx {
        fluid_log(FLUID_ERR, "Preset header indices not monotonic");
        return false;
    }

    let i2 = (pbag_idx - prev_pbag_idx) as usize;

    let prev = prev_preset.expect("at least one preset");
    if !add_placeholders(&mut sf.preset[prev].zone, i2) {
        return false;
    }

    true
}

/// Translation of `load_pbag()`: preset bag loader.
fn load_pbag(sf: &mut SfData, size: i32) -> bool {
    load_bag(sf, size, true)
}

/// `load_pbag()` and `load_ibag()`, which only differ in their lists and
/// messages.
fn load_bag(sf: &mut SfData, mut size: i32, preset: bool) -> bool {
    // (pz: the previous zone, as (preset or instrument, zone) indices)
    let mut pz: Option<(usize, usize)> = None;
    let mut pgenndx: u16 = 0;
    let mut pmodndx: u16 = 0;

    if size % SF_BAG_SIZE != 0 || size == 0 {
        /* size is multiple of SF_BAG_SIZE? */
        fluid_log(
            FLUID_ERR,
            if preset {
                "Preset bag chunk size is invalid"
            } else {
                "Instrument bag chunk size is invalid"
            },
        );
        return false;
    }

    let nitems = if preset {
        sf.preset.len()
    } else {
        sf.inst.len()
    };

    /* traverse through presets */
    for p in 0..nitems {
        let nzones = if preset {
            sf.preset[p].zone.len()
        } else {
            sf.inst[p].zone.len()
        };

        /* traverse preset's zones */
        for z in 0..nzones {
            size -= SF_BAG_SIZE;
            if size < 0 {
                fluid_log(
                    FLUID_ERR,
                    if preset {
                        "Preset bag chunk size mismatch"
                    } else {
                        "Instrument bag chunk size mismatch"
                    },
                );
                return false;
            }

            if preset {
                sf.preset[p].zone[z] = Some(SfZone::default());
            } else {
                sf.inst[p].zone[z] = Some(SfZone::default());
            }
            let genndx = try_read!(sf.readw()); /* possible read failure ^ */
            let modndx = try_read!(sf.readw());

            if let Some(pz) = pz {
                /* if not first zone */
                if genndx < pgenndx {
                    fluid_log(
                        FLUID_ERR,
                        if preset {
                            "Preset bag generator indices not monotonic"
                        } else {
                            "Instrument generator indices not monotonic"
                        },
                    );
                    return false;
                }

                if modndx < pmodndx {
                    fluid_log(
                        FLUID_ERR,
                        if preset {
                            "Preset bag modulator indices not monotonic"
                        } else {
                            "Instrument modulator indices not monotonic"
                        },
                    );
                    return false;
                }

                if !bag_placeholders(sf, preset, pz, genndx - pgenndx, modndx - pmodndx) {
                    return false;
                }
            }

            pz = Some((p, z)); /* update previous zone ptr */
            pgenndx = genndx; /* update previous zone gen index */
            pmodndx = modndx; /* update previous zone mod index */
        }
    }

    size -= SF_BAG_SIZE;

    if size != 0 {
        fluid_log(
            FLUID_ERR,
            if preset {
                "Preset bag chunk size mismatch"
            } else {
                "Instrument chunk size mismatch"
            },
        );
        return false;
    }

    let genndx = try_read!(sf.readw());
    let modndx = try_read!(sf.readw());

    let Some(pz) = pz else {
        if genndx > 0 {
            fluid_log(
                FLUID_WARN,
                if preset {
                    "No preset generators and terminal index not 0"
                } else {
                    "No instrument generators and terminal index not 0"
                },
            );
        }

        if modndx > 0 {
            fluid_log(
                FLUID_WARN,
                if preset {
                    "No preset modulators and terminal index not 0"
                } else {
                    "No instrument modulators and terminal index not 0"
                },
            );
        }

        return true;
    };

    if genndx < pgenndx {
        fluid_log(
            FLUID_ERR,
            if preset {
                "Preset bag generator indices not monotonic"
            } else {
                "Instrument generator indices not monotonic"
            },
        );
        return false;
    }

    if modndx < pmodndx {
        fluid_log(
            FLUID_ERR,
            if preset {
                "Preset bag modulator indices not monotonic"
            } else {
                "Instrument modulator indices not monotonic"
            },
        );
        return false;
    }

    bag_placeholders(sf, preset, pz, genndx - pgenndx, modndx - pmodndx)
}

/// The generator and modulator placeholders of a preset or instrument
/// zone.
fn bag_placeholders(
    sf: &mut SfData,
    preset: bool,
    (p, z): (usize, usize),
    ngen: u16,
    nmod: u16,
) -> bool {
    let zone = if preset {
        sf.preset[p].zone[z].as_mut()
    } else {
        sf.inst[p].zone[z].as_mut()
    };
    let zone = zone.expect("zone loaded");
    add_placeholders(&mut zone.gen, ngen as usize)
        && add_placeholders(&mut zone.mod_, nmod as usize)
}

/// Translation of `load_pmod()`: preset modulator loader.
fn load_pmod(sf: &mut SfData, size: i32) -> bool {
    load_mod(sf, size, true)
}

/// `load_pmod()` and `load_imod()`, which only differ in their lists and
/// messages.
fn load_mod(sf: &mut SfData, mut size: i32, preset: bool) -> bool {
    let mismatch = if preset {
        "Preset modulator chunk size mismatch"
    } else {
        "Instrument modulator chunk size mismatch"
    };
    let nitems = if preset {
        sf.preset.len()
    } else {
        sf.inst.len()
    };

    for p in 0..nitems {
        /* traverse through all presets */
        let nzones = if preset {
            sf.preset[p].zone.len()
        } else {
            sf.inst[p].zone.len()
        };

        for z in 0..nzones {
            /* traverse this preset's zones */
            let nmods = zone_mut(sf, preset, p, z).mod_.len();

            for m in 0..nmods {
                /* load zone's modulators */
                size -= SF_MOD_SIZE;
                if size < 0 {
                    fluid_log(FLUID_ERR, mismatch);
                    return false;
                }

                let src = try_read!(sf.readw());
                let dest = try_read!(sf.readw());
                let amount = try_read!(sf.readw()) as i16;
                let amtsrc = try_read!(sf.readw());
                let trans = try_read!(sf.readw());
                zone_mut(sf, preset, p, z).mod_[m] = Some(SfMod {
                    src,
                    dest,
                    amount,
                    amtsrc,
                    trans,
                });
            }
        }
    }

    /*
      If there isn't even a terminal record
      Hmmm, the specs say there should be one, but..
    */
    if size == 0 {
        return true;
    }

    size -= SF_MOD_SIZE;

    if size != 0 {
        fluid_log(FLUID_ERR, mismatch);
        return false;
    }

    try_skip!(sf.fskip(SF_MOD_SIZE as i64)); /* terminal mod */

    true
}

/// A preset's or instrument's zone, which load_pbag()/load_ibag() filled.
fn zone_mut(sf: &mut SfData, preset: bool, p: usize, z: usize) -> &mut SfZone {
    let zone = if preset {
        sf.preset[p].zone[z].as_mut()
    } else {
        sf.inst[p].zone[z].as_mut()
    };
    // (every zone placeholder gets a zone in load_pbag()/load_ibag(), or
    // the loading fails there)
    zone.expect("zone loaded")
}

/// Translation of `find_gen_by_id()`: find a generator by its id in the
/// passed in list.
///
/// @return the index of the SFGen if found, otherwise NULL
fn find_gen_by_id(gen: u16, genlist: &[Option<SfGen>]) -> Option<usize> {
    /* is generator in gen list? */
    for (i, p) in genlist.iter().enumerate() {
        let p = (*p)?;

        if gen == p.id {
            return Some(i);
        }
    }

    None
}

/* -------------------------------------------------------------------
 * preset generator loader
 * generator (per preset) loading rules:
 * Zones with no generators or modulators shall be annihilated
 * Global zone must be 1st zone, discard additional ones (instrumentless zones)
 *
 * generator (per zone) loading rules (in order of decreasing precedence):
 * KeyRange is 1st in list (if exists), else discard
 * if a VelRange exists only preceded by a KeyRange, else discard
 * if a generator follows an instrument discard it
 * if a duplicate generator exists replace previous one
 * ------------------------------------------------------------------- */
/// Translation of `load_pgen()`.
pub(crate) fn load_pgen(sf: &mut SfData, size: i32) -> bool {
    load_gen(sf, size, true)
}

/// `load_pgen()` and `load_igen()` (see load_pgen for loading rules),
/// which differ in their lists, their last generator (instrument or
/// sample), the generators valid in them and the messages.
fn load_gen(sf: &mut SfData, mut size: i32, preset: bool) -> bool {
    let kind = if preset { "preset" } else { "instrument" };
    let nitems = if preset {
        sf.preset.len()
    } else {
        sf.inst.len()
    };

    for p in 0..nitems {
        let name = if preset {
            sf.preset[p].name.clone()
        } else {
            sf.inst[p].name.clone()
        };

        /* traverse through all presets */
        let mut discarded = false;
        let mut z = 0;
        let mut zi = 0; /* zone_list */

        /* traverse preset's zones */
        loop {
            let nzones = if preset {
                sf.preset[p].zone.len()
            } else {
                sf.inst[p].zone.len()
            };
            if zi >= nzones {
                break;
            }

            let mut level = 0;
            let mut gi = 0; /* gen_list */

            loop {
                if gi >= zone_mut(sf, preset, p, zi).gen.len() {
                    break;
                }

                /* load zone's generators */
                let mut dup: Option<usize> = None;
                let mut skip = false;
                let mut drop = false;
                let mut genval = SfGenAmount(0);

                size -= SF_GEN_SIZE;
                if size < 0 {
                    fluid_log(
                        FLUID_ERR,
                        if preset {
                            "Preset generator chunk size mismatch, rejecting SoundFont as structurally defective."
                        } else {
                            "IGEN chunk size mismatch, rejecting SoundFont as structurally defective."
                        },
                    );
                    return false;
                }

                let genid = try_read!(sf.readw());

                if genid == GEN_KEYRANGE as u16 {
                    /* nothing precedes */
                    if level == 0 {
                        level = 1;
                        let lo = try_read!(sf.readb());
                        let hi = try_read!(sf.readb());
                        genval = SfGenAmount(u16::from_le_bytes([lo, hi]));
                    } else {
                        skip = true;
                        flog!(
                            FLUID_DBG,
                            "Discarding out of order generator KeyRange in {} '{}' of zone {} (must be the first generator per SoundFont spec 8.1.2).",
                            kind,
                            name,
                            z
                        );
                    }
                } else if genid == GEN_VELRANGE as u16 {
                    /* only KeyRange precedes */
                    if level <= 1 {
                        level = 2;
                        let lo = try_read!(sf.readb());
                        let hi = try_read!(sf.readb());
                        genval = SfGenAmount(u16::from_le_bytes([lo, hi]));
                    } else {
                        skip = true;
                        flog!(
                            FLUID_DBG,
                            "Discarding out of order generator VelRange in {} '{}' of zone {} (must only be preceded by the KeyRange generator per SoundFont spec 8.1.2).",
                            kind,
                            name,
                            z
                        );
                    }
                } else if preset && genid == GEN_INSTRUMENT as u16 {
                    /* inst is last gen */
                    level = 3;
                    genval = SfGenAmount(try_read!(sf.readw()));
                } else if !preset && genid == GEN_SAMPLEID as u16 {
                    /* sample is last gen */
                    level = 3;
                    genval = SfGenAmount(try_read!(sf.readw()));

                    if gi + 1 < zone_mut(sf, preset, p, zi).gen.len() {
                        flog!(
                            FLUID_DBG,
                            "Generator SampleID in instrument '{}' of zone {} is followed by additional generators. Per SoundFont spec 8.1.2, SampleID must be the last generator in the zone. All subsequent generators will be discarded.",
                            name,
                            z
                        );
                    }
                } else {
                    level = 2;

                    let valid = if preset {
                        valid_preset_genid(genid)
                    } else {
                        valid_inst_genid(genid)
                    };
                    if valid {
                        /* generator valid? */
                        genval = SfGenAmount(try_read!(sf.readw()));
                        dup = find_gen_by_id(genid, &zone_mut(sf, preset, p, zi).gen);

                        if dup.is_some() {
                            flog!(
                                FLUID_DBG,
                                "Duplicate generator {} in {} '{}' of zone {} found. Only the last occurrence is kept, previous one(s) will be dropped.",
                                fluid_gen_name(genid as i32),
                                kind,
                                name,
                                z
                            );
                        }
                    } else {
                        skip = true;
                        if preset {
                            flog!(
                                FLUID_DBG,
                                "Generator {} encountered in preset '{}' of zone {} is either not valid for a preset zone or unknown; discarding.",
                                fluid_gen_name(genid as i32),
                                name,
                                z
                            );
                        } else {
                            flog!(
                                FLUID_DBG,
                                "Generator {} in instrument '{}' of zone {} is invalid for instruments per SoundFont spec 8.1.2. Discarding.",
                                fluid_gen_name(genid as i32),
                                name,
                                z
                            );
                        }
                    }
                }

                if !skip {
                    let zone = zone_mut(sf, preset, p, zi);
                    match dup {
                        None => {
                            /* if gen ! dup alloc new */
                            zone.gen[gi] = Some(SfGen {
                                id: genid,
                                amount: genval,
                            });
                        }
                        Some(d) => {
                            /* ptr to orig gen */
                            if let Some(g) = zone.gen[d].as_mut() {
                                g.amount = genval;
                            }
                            drop = true;
                        }
                    }
                } else {
                    /* Skip this generator */
                    discarded = true;
                    drop = true;
                    try_skip!(sf.fskipw());
                }

                if !preset && drop && dup.is_some() {
                    flog!(
                        FLUID_DBG,
                        "Dropping previous instance of generator {} in instrument '{}' of zone {} due to duplicate definition (last one kept).",
                        fluid_gen_name(genid as i32),
                        name,
                        z
                    );
                }

                if !drop {
                    gi += 1; /* next gen */
                } else {
                    zone_mut(sf, preset, p, zi).gen.remove(gi); /* drop place holder */
                }

                /* GEN_INSTRUMENT should be the last generator */
                if level == 3 {
                    break;
                }
            } /* generator loop */

            /* Anything below level 3 means it's a global zone. The global zone
             * should always be the first zone in the list, so discard any
             * other global zones we encounter */
            if level < 3 && zi != 0 {
                /* advance to next zone before deleting the current list element */
                if preset {
                    flog!(
                        FLUID_WARN,
                        "Preset '{}': Discarding invalid global zone (global zones must appear first in presets per SoundFont spec 7.3)",
                        name
                    );
                    sf.preset[p].zone.remove(zi);
                } else {
                    flog!(
                        FLUID_WARN,
                        "Instrument '{}': Discarding invalid global zone (global zones must appear first in instruments per SoundFont spec 7.7).",
                        name
                    );
                    sf.inst[p].zone.remove(zi);
                }

                z += 1;
                /* we have already advanced the zone_list pointer, so continue with next zone */
                continue;
            }

            /* All remaining generators are invalid and should be discarded
             * (because they come after an instrument generator) */
            while gi < zone_mut(sf, preset, p, zi).gen.len() {
                discarded = true;

                size -= SF_GEN_SIZE;
                if size < 0 {
                    fluid_log(
                        FLUID_ERR,
                        if preset {
                            "Preset generator chunk size mismatch, rejecting SoundFont as structurally defective."
                        } else {
                            "Instrument generator chunk size mismatch, rejecting SoundFont as structurally defective."
                        },
                    );
                    return false;
                }

                if let Some(g) = zone_mut(sf, preset, p, zi).gen[gi] {
                    flog!(
                        FLUID_DBG,
                        "Generator {} in {} '{}' of zone {} appears after SampleID, in violation of SoundFont spec 8.1.2; discarding.",
                        fluid_gen_name(g.id as i32),
                        kind,
                        name,
                        z
                    );
                } else {
                    flog!(
                        FLUID_DBG,
                        "Empty generator in {} '{}' of zone {}; discarding.",
                        kind,
                        name,
                        z
                    );
                }

                try_skip!(sf.fskip(SF_GEN_SIZE as i64));
                zone_mut(sf, preset, p, zi).gen.remove(gi);
            }

            z += 1;
            zi += 1;
        }

        if discarded {
            if preset {
                flog!(
                    FLUID_WARN,
                    "Preset '{}': Some invalid generators were discarded, potentially resulting in a different timbre. Run fluidsynth in verbose mode for detailed information.",
                    name
                );
            } else {
                flog!(
                    FLUID_WARN,
                    "Instrument '{}': Some invalid generators were discarded, audible glitches are to be expected! Run fluidsynth in verbose mode for detailed information.",
                    name
                );
            }
        }
    }

    /* in case there isn't a terminal record */
    if size == 0 {
        return true;
    }

    size -= SF_GEN_SIZE;

    if size != 0 {
        fluid_log(
            FLUID_ERR,
            if preset {
                "Preset generator chunk size mismatch"
            } else {
                "IGEN chunk size mismatch"
            },
        );
        return false;
    }

    try_skip!(sf.fskip(SF_GEN_SIZE as i64)); /* terminal gen */

    true
}

/// Translation of `load_ihdr()`: instrument header loader.
fn load_ihdr(sf: &mut SfData, mut size: u32) -> bool {
    let mut prev_inst: Option<usize> = None; /* ptr to current & previous instrument */
    let mut pzndx: u16 = 0;

    if !size.is_multiple_of(SF_IHDR_SIZE) || size == 0 {
        /* chunk size is valid? */
        fluid_log(FLUID_ERR, "Instrument header has invalid size");
        return false;
    }

    size = size / SF_IHDR_SIZE - 1;

    if size == 0 {
        /* at least one preset + term record */
        fluid_log(FLUID_WARN, "File contains no instruments");
        try_skip!(sf.fskip(SF_IHDR_SIZE as i64));
        return true;
    }

    for i in 0..size {
        /* load all instrument headers */
        sf.inst.push(SfInst {
            idx: i as i32,
            ..Default::default()
        });
        let cur = sf.inst.len() - 1;
        sf.inst[cur].name = try_read!(sf.readstr()); /* Possible read failure ^ */
        let zndx = try_read!(sf.readw());

        if let Some(prev) = prev_inst {
            /* not first instrument? */
            if zndx < pzndx {
                fluid_log(FLUID_ERR, "Instrument header indices not monotonic");
                return false;
            }

            let i2 = (zndx - pzndx) as usize;

            if !add_placeholders(&mut sf.inst[prev].zone, i2) {
                return false;
            }
        } else if zndx > 0 {
            /* 1st inst, warn if ofs >0 */
            flog!(
                FLUID_WARN,
                "{} instrument zones not referenced, discarding",
                zndx
            );
        }

        pzndx = zndx;
        prev_inst = Some(cur); /* update instrument ptr */
    }

    try_skip!(sf.fskip(20));
    let zndx = try_read!(sf.readw());

    if zndx < pzndx {
        fluid_log(FLUID_ERR, "Instrument header indices not monotonic");
        return false;
    }

    let i2 = (zndx - pzndx) as usize;

    let prev = prev_inst.expect("at least one instrument");
    if !add_placeholders(&mut sf.inst[prev].zone, i2) {
        return false;
    }

    true
}

/// Translation of `load_ibag()`: instrument bag loader.
fn load_ibag(sf: &mut SfData, size: i32) -> bool {
    load_bag(sf, size, false)
}

/// Translation of `load_imod()`: instrument modulator loader.
fn load_imod(sf: &mut SfData, size: i32) -> bool {
    load_mod(sf, size, false)
}

/// Translation of `load_igen()`: load instrument generators (see
/// load_pgen for loading rules).
pub(crate) fn load_igen(sf: &mut SfData, size: i32) -> bool {
    load_gen(sf, size, false)
}

/// Translation of `load_shdr()`: sample header loader.
fn load_shdr(sf: &mut SfData, mut size: u32) -> bool {
    if !size.is_multiple_of(SF_SHDR_SIZE) || size == 0 {
        /* size is multiple of SHDR size? */
        fluid_log(FLUID_ERR, "Sample header has invalid size");
        return false;
    }

    size = size / SF_SHDR_SIZE - 1;

    if size == 0 {
        /* at least one sample + term record? */
        fluid_log(FLUID_WARN, "File contains no samples");
        try_skip!(sf.fskip(SF_SHDR_SIZE as i64));
        return true;
    }

    /* load all sample headers */
    for i in 0..size {
        let mut p = SfSample {
            idx: i as i32,
            ..Default::default()
        };

        // (fluid_list_prepend(): the list is in reverse file order)
        p.name = try_read!(sf.readstr());
        p.start = try_read!(sf.readd());
        p.end = try_read!(sf.readd());
        p.loopstart = try_read!(sf.readd());
        p.loopend = try_read!(sf.readd());
        p.samplerate = try_read!(sf.readd());
        p.origpitch = try_read!(sf.readb());
        p.pitchadj = try_read!(sf.readb()) as i8;
        try_skip!(sf.fskipw()); /* skip sample link */
        p.sampletype = try_read!(sf.readw());
        sf.sample.insert(0, p);
    }

    try_skip!(sf.fskip(SF_SHDR_SIZE as i64)); /* skip terminal shdr */

    true
}

/// Translation of `preset_compare_func()`: preset sort function, first by
/// bank, then by preset #.
fn preset_compare_func(a: &SfPreset, b: &SfPreset) -> i32 {
    let aval = ((a.bank as i32) << 16) | a.prenum as i32;
    let bval = ((b.bank as i32) << 16) | b.prenum as i32;

    aval.wrapping_sub(bval)
}

/// Translation of `valid_inst_genid()`: check validity of instrument
/// generator.
fn valid_inst_genid(genid: u16) -> bool {
    /* OVERRIDEROOTKEY is the last official generator, everything
     * following it are generators internal to FluidSynth and will
     * never appear in a SoundFont file. */
    if genid > GEN_OVERRIDEROOTKEY as u16 {
        return false;
    }

    !INVALID_INST_GEN.contains(&genid)
}

/// Translation of `valid_preset_genid()`: check validity of preset
/// generator.
fn valid_preset_genid(genid: u16) -> bool {
    if !valid_inst_genid(genid) {
        return false;
    }

    !INVALID_PRESET_GEN.contains(&genid)
}

/// Reads `bytes` bytes with the file callbacks, growing the buffer as the
/// data arrives (in steps of at most 1 MiB), so that a size from a
/// corrupt header doesn't allocate (or zero) more memory than the file
/// has. Returns None if the allocation or a read fails (the callbacks'
/// read is all or nothing, so a short file fails the same as upstream's
/// single read, only perhaps a step later).
fn read_bytes(sf: &mut SfData, bytes: usize) -> Option<Vec<u8>> {
    const STEP: usize = 1 << 20;
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < bytes {
        let n = (bytes - buf.len()).min(STEP);
        let at = buf.len();
        if buf.try_reserve_exact(n).is_err() {
            return None;
        }
        buf.resize(at + n, 0);
        if !sf.fread(&mut buf[at..]) {
            return None;
        }
    }
    Some(buf)
}

/// Translation of `fluid_sffile_read_wav()`.
fn fluid_sffile_read_wav(
    sf: &mut SfData,
    start: u32,
    end: u32,
) -> Option<(Vec<i16>, Option<Vec<u8>>)> {
    if end.wrapping_add(1) <= start {
        /* fluid_return_val_if_fail((end + 1) > start, -1); */
        return None;
    }

    let num_samples = end.wrapping_add(1) - start;

    if (start as u64 * 2 > sf.samplesize as u64) || (end as u64 * 2 > sf.samplesize as u64) {
        fluid_log(FLUID_ERR, "Sample offsets exceed sample data chunk");
        return None;
    }

    /* Load 16-bit sample data */
    if !sf.fseek(sf.samplepos as i64 + (start as i64 * 2), SEEK_SET) {
        fluid_log(FLUID_ERR, "Failed to seek to sample position");
        return None;
    }

    // (FLUID_ARRAY(short, num_samples) and the read, see read_bytes())
    let bytes = num_samples as u64 * 2;
    let raw = if bytes > usize::MAX as u64 {
        None
    } else {
        read_bytes(sf, bytes as usize)
    };
    let Some(raw) = raw else {
        fluid_log(FLUID_ERR, "Failed to read sample data");
        return None;
    };

    /* If this machine is big endian, byte swap the 16 bit samples */
    let loaded_data: Vec<i16> = raw
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();

    /* Optionally load additional 8 bit sample data for 24-bit support. Any failures while loading
     * the 24-bit sample data will be logged as errors but won't prevent the sample reading to
     * fail, as sound output is still possible with the 16-bit sample data. */
    let mut loaded_data24 = None;
    if sf.sample24pos != 0 {
        'error24: {
            if (start > sf.sample24size) || (end > sf.sample24size) {
                fluid_log(FLUID_ERR, "Sample offsets exceed 24-bit sample data chunk");
                break 'error24;
            }

            if !sf.fseek(sf.sample24pos as i64 + start as i64, SEEK_SET) {
                fluid_log(
                    FLUID_ERR,
                    "Failed to seek position for 24-bit sample data in data file",
                );
                break 'error24;
            }

            let Some(data24) = read_bytes(sf, num_samples as usize) else {
                fluid_log(FLUID_ERR, "Failed to read 24-bit sample data");
                break 'error24;
            };

            loaded_data24 = Some(data24);
        }

        if loaded_data24.is_none() {
            fluid_log(
                FLUID_WARN,
                "Ignoring 24-bit sample data, sound quality might suffer",
            );
        }
    }

    Some((loaded_data, loaded_data24))
}

/// Translation of `fluid_sffile_read_vorbis()`: Ogg Vorbis loading and
/// decompression, which fails without libsndfile.
fn fluid_sffile_read_vorbis(
    _sf: &mut SfData,
    _start_byte: u32,
    _end_byte: u32,
) -> Option<(Vec<i16>, Option<Vec<u8>>)> {
    None
}
