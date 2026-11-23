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
// (fluid_dls.cpp has no license notice of its own; it is part of
// FluidSynth, under the license above: see COPYING.)
// Modified 2026-10-17: translated into Rust from the part of
// fluid_dls.cpp in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE) that
// SDL_mixer reaches: fluid_dls_loader_load() failing to open the file
// (SDL_mixer's SoundFont names only open through its own loader).
// Modified 2026-11-23: the rest of fluid_dls.cpp translated, in its order:
// the exceptions (`DlsException`, carried in `Result`) and log_exception(),
// the RIFF chunk reading, the DLSIDs and the conditional chunks (CDL), the
// connection blocks, the wave pool, the instruments, their regions and
// articulations, the MobileBAE aliasing ('pgal'), the loader with its
// default modulators, and the SoundFont and preset methods with the
// noteon. The libsndfile path is left out, as SDL_mixer's CMake builds
// FluidSynth without libsndfile; so is pinning the sample data to RAM
// (mlock).

use std::collections::TryReserveError;
use std::sync::Arc;

use crate::fluidsynth_h::*;
use crate::fluidsynth_priv::c_double_to_int;
use crate::sfloader::defsfont::{fluid_zone_inside_range, FluidZoneRange};
use crate::sfloader::sfont::{
    fluid_sample_sanitize_loop, fluid_sample_validate, FluidFileCallbacks, FluidSfont, Sample,
    SfFile, SfLoader, SfontData,
};
use crate::synth::gen::*;
use crate::synth::modulator::{fluid_mod_test_identity, FluidMod};
use crate::synth::synth::{
    FluidSynthInner, CUSTOM_BALANCE_MOD, DEFAULT_ATT_MOD, DEFAULT_EXPR_MOD, DEFAULT_MOD2VIBLFO_MOD,
    DEFAULT_PAN_MOD, DEFAULT_VEL2ATT_MOD, DLS_DEFAULT_CHORUS_MOD, DLS_DEFAULT_PITCH_BEND_MOD,
    DLS_DEFAULT_REVERB_MOD, DRUM_INST_BANK, FLUID_BANK_STYLE_GM, FLUID_BANK_STYLE_GS,
    FLUID_BANK_STYLE_MMA, FLUID_BANK_STYLE_XG,
};
use crate::synth::voice::{fluid_voice_add_mod_local, fluid_voice_gen_incr, fluid_voice_gen_set};
use crate::utils::sys::{
    flog, FLUID_DBG, FLUID_ERR, FLUID_PANIC, FLUID_WARN, SEEK_CUR, SEEK_END, SEEK_SET,
};
use crate::FluidReal;

// (`string_format()`, based on a StackOverflow answer under CC0 1.0, is
// `format!` here: the messages it makes keep the NUL characters a `%c`
// gives them, and are cut at the first one where C prints them, see
// `c_str()`.)

/// The kind of a C++ exception the loader throws (each kind's `what()` is
/// libstdc++'s, the reference toolchain's C++ library).
#[derive(Debug)]
enum DlsExceptionKind {
    /// `std::runtime_error` with its message.
    Runtime(String),
    /// `std::exception{}`.
    Std,
    /// `std::bad_alloc`, from an allocation sized from the file.
    BadAlloc,
    /// `std::out_of_range` with its message (from `vector::at()`).
    OutOfRange(String),
}

/// A C++ exception the loader throws, with the one
/// `std::throw_with_nested()` nested in it (if any).
#[derive(Debug)]
pub(crate) struct DlsException {
    kind: DlsExceptionKind,
    nested: Option<Box<DlsException>>,
}

impl DlsException {
    /// `std::runtime_error{ message }`.
    pub(crate) fn runtime(message: impl Into<String>) -> DlsException {
        DlsException {
            kind: DlsExceptionKind::Runtime(message.into()),
            nested: None,
        }
    }

    /// `std::exception{}`.
    pub(crate) fn std() -> DlsException {
        DlsException {
            kind: DlsExceptionKind::Std,
            nested: None,
        }
    }

    /// `std::bad_alloc` (what an allocation the file sizes throws).
    pub(crate) fn bad_alloc() -> DlsException {
        DlsException {
            kind: DlsExceptionKind::BadAlloc,
            nested: None,
        }
    }

    /// The `std::out_of_range` that `vector::at(n)` throws for a vector of
    /// `size` elements.
    fn out_of_range(n: usize, size: usize) -> DlsException {
        DlsException {
            kind: DlsExceptionKind::OutOfRange(format!(
                "vector::_M_range_check: __n (which is {}) >= this->size() (which is {})",
                n, size
            )),
            nested: None,
        }
    }

    /// `std::throw_with_nested(this)` in the `catch` of `exc`.
    pub(crate) fn nest(mut self, exc: DlsException) -> DlsException {
        self.nested = Some(Box::new(exc));
        self
    }

    /// `what()`.
    fn what(&self) -> &str {
        match &self.kind {
            DlsExceptionKind::Runtime(message) => message,
            DlsExceptionKind::Std => "std::exception",
            DlsExceptionKind::BadAlloc => "std::bad_alloc",
            DlsExceptionKind::OutOfRange(message) => message,
        }
    }
}

impl From<TryReserveError> for DlsException {
    /// The `std::bad_alloc` of a failed allocation.
    fn from(_: TryReserveError) -> DlsException {
        DlsException::bad_alloc()
    }
}

/// A result or a C++ exception thrown.
pub(crate) type DlsResult<T> = Result<T, DlsException>;

/// A message as C prints it with `%s`: up to its first NUL.
fn c_str(message: &str) -> &str {
    message.split('\0').next().unwrap_or("")
}

/// Translation of `log_exception()`.
pub(crate) fn log_exception(level: i32, exc: &DlsException, nest_level: i32) {
    let mut prefix = String::new();

    if nest_level > 0 {
        prefix = " ".repeat(nest_level as usize);
    }

    if let DlsExceptionKind::BadAlloc = exc.kind {
        flog!(
            FLUID_PANIC,
            "{}Out of memory!!: {}",
            prefix,
            c_str(exc.what())
        );
    } else {
        flog!(level, "{}exc: {}", prefix, c_str(exc.what()));
    }

    /* std::rethrow_if_nested() */
    if let Some(nested) = &exc.nested {
        if nest_level + 1 >= 16 {
            flog!(level, "{}...", prefix);
            return;
        }

        log_exception(level, nested, nest_level + 1);
    }
}

/// `FMT_4CC_SPEC` with `FMT_4CC_ARG(x)`: a FOURCC's bytes, each as `%c`
/// prints it.
fn fmt_4cc(x: u32) -> String {
    x.to_le_bytes().iter().map(|&b| b as char).collect()
}

/// Translation of `RIFFChunk`.
#[derive(Clone, Copy, Debug, Default)]
struct RiffChunk {
    id: u32,   // char[4]
    size: u32, // native endian
}

// (`scope_guard` closing the file is `DlsFont`'s `Drop`. `mlock_guard` is
// left out: see the constructor.)

// internal struct for keeping some nice information of the DLS

/// Translation of `fluid_dls_articulation`: note that it is at the level
/// of lart (one fluid_dls_articulation per instrument (region)).
#[derive(Clone, Debug)]
pub(crate) struct DlsArticulation {
    gens: [Option<FluidReal>; GEN_LAST],
    mods: Vec<FluidMod>,

    // Scale of the Key Num Generator, "MIDI Note to Key", divided by 12800.
    // note that this is only valid at instrument level, as it affects region selection process.
    keynum_scale: FluidReal,

    // DLS articulator Key Number to Pitch (default 12800 cents)
    key_to_pitch: FluidMod,
}

impl Default for DlsArticulation {
    fn default() -> DlsArticulation {
        let mut articulation = DlsArticulation {
            gens: [None; GEN_LAST],
            mods: Vec::new(),
            keynum_scale: 1.0,
            key_to_pitch: FluidMod {
                dest: GEN_FINETUNE as u8,
                src1: FLUID_MOD_KEY,
                flags1: FLUID_MOD_GC | FLUID_MOD_LINEAR | FLUID_MOD_UNIPOLAR | FLUID_MOD_POSITIVE,
                src2: 0,
                flags2: 0,
                trans: FLUID_MOD_TRANSFORM_LINEAR,
                amount: 12800.0,
            },
        };

        // overriding default gens

        articulation.gens[GEN_MODLFOFREQ] = Some(-851.3179423647571); // 5 Hz
        articulation.gens[GEN_MODLFODELAY] = Some(-7972.627427729669); // 10 ms
        articulation.gens[GEN_VIBLFOFREQ] = Some(-851.3179423647571); // 5 Hz
        articulation.gens[GEN_VIBLFODELAY] = Some(-7972.627427729669); // 10 ms
        articulation
    }
}

impl DlsArticulation {
    /// `add_mod()`: an identical modulator's amount is replaced.
    fn add_mod(&mut self, mod_: &FluidMod) {
        if fluid_mod_test_identity(&self.key_to_pitch, mod_) {
            self.key_to_pitch.amount = mod_.amount;
            return;
        }

        for existing_mod in &mut self.mods {
            if fluid_mod_test_identity(existing_mod, mod_) {
                existing_mod.amount = mod_.amount;
                return;
            }
        }

        self.mods.push(*mod_);
    }
}

/// Translation of `fluid_dls_wsmp`: wsmp is used in LIST[wave] chunk and
/// LIST[rgn ] chunk, converting to different fluid structures.
#[derive(Clone, Copy, Debug, Default)]
struct DlsWsmp {
    unity_note: u16,
    fine_tune: i16, // in cents
    gain: i32,      // in (cB/65536)
    loop_type: u32,
    loop_start: u32,
    loop_length: u32,
}

/// Translation of `fluid_dls_sample`.
#[derive(Debug, Default)]
struct DlsSample {
    // N.B. wlnk is at region level in DLS, but sample level in SF2 (FluidSynth)
    // as wave link is not implemented now, it is simply ignored.
    name: String,
    samplerate: u32,
    start: u32,
    end: u32, // past the end
    wsmp: Option<DlsWsmp>,
}

/// Translation of `fluid_dls_region`.
#[derive(Debug)]
struct DlsRegion {
    range: Arc<FluidZoneRange>,
    wsmp: Option<DlsWsmp>,
    exclusive_class: i32,

    artindex: u32,
    sampleindex: u32,
    samplemode_inherited: u8, // loop_type from sample's wsmp, converted to SF2 sample mode
    gain_inherited: FluidReal, // gain from sample's wsmp, in cB
}

impl Default for DlsRegion {
    fn default() -> DlsRegion {
        DlsRegion {
            range: Arc::default(),
            wsmp: None,
            exclusive_class: 0,
            artindex: u32::MAX, // -1
            sampleindex: 0,
            samplemode_inherited: 0,
            gain_inherited: 0.0,
        }
    }
}

/// Translation of `fluid_dls_instrument_alias`.
#[derive(Clone, Copy, Debug)]
struct DlsInstrumentAlias {
    pcnum: i32,
    bankmsb: i32,
    banklsb: i32,
}

/// Translation of `fluid_dls_instrument`: SF2's instrument level does not
/// exist in DLS, preset is called instrument in DLS. (Upstream's synth
/// pointer is the `bank_select` its methods are given.)
#[derive(Debug)]
pub(crate) struct DlsInstrument {
    pcnum: i32,
    bankmsb: i32,
    banklsb: i32,
    is_drums: bool,

    name: String,
    regions: Vec<DlsRegion>,

    samples_fluid: Arc<[Arc<Sample>]>,
    articulations: Arc<[DlsArticulation]>,
    drum_note_aliasing: Option<Arc<[u8; 128]>>,

    // see fluid_dls_articulation::keynum_scale
    keynum_scale: FluidReal,

    // for 'pgal' chunk in MobileBAE DLS banks
    aliases: Vec<DlsInstrumentAlias>,
}

impl Default for DlsInstrument {
    fn default() -> DlsInstrument {
        DlsInstrument {
            pcnum: 0,
            bankmsb: 0,
            banklsb: 0,
            is_drums: false,
            name: String::new(),
            regions: Vec::new(),
            samples_fluid: Arc::default(),
            articulations: Arc::default(),
            drum_note_aliasing: None,
            keynum_scale: 1.0,
            aliases: Vec::new(),
        }
    }
}

/// Translation of `fluid_dls_instrument_fluid_data`: a preset, which is an
/// instrument or one of its aliases (the `fluid_preset_t` whose user data
/// this is, is the `FluidPreset` holding it).
#[derive(Debug)]
pub(crate) struct DlsPreset {
    pcnum: i32,
    bankmsb: i32,
    banklsb: i32,
    is_drums: bool,

    instrument: Arc<DlsInstrument>, // backing instrument
}

/// Translation of `fluid_dls_font`. (Its file callbacks are the methods of
/// its open `file`.)
pub(crate) struct DlsFont {
    output_sample_rate: u32, // only used for cdl eval
    filename: String,
    /* (the SoundFont's default modulators, which the samples get) */
    default_mod_list: Option<Arc<Vec<FluidMod>>>,

    /* (closed when the font drops, as upstream's `on_file_exit`) */
    file: Option<Box<dyn SfFile>>,
    filesize: i64,

    // offset to chunk header
    linsoffset: i64,
    wvploffset: i64,

    // offset to chunk data
    pgaloffset: i64,
    pgalsize: i64,

    sampledata: Vec<i16>,
    poolcues: Vec<u32>, // data of ptbl

    samples: Vec<DlsSample>,
    articulations: Vec<DlsArticulation>,
    // (as parsed and sorted: the presets hold them once they are made)
    instruments: Vec<DlsInstrument>,
    /* (instruments_fluid_data: each instrument, then its aliases) */
    presets: Vec<Arc<DlsPreset>>,
    samples_fluid: Arc<[Arc<Sample>]>,

    // for 'pgal' chunk in MobileBAE DLS banks
    drum_note_aliasing: Option<[u8; 128]>,

    fluid_preset_iterator: usize,

    total_presets: i64, // total number of presets, including aliases
}

impl std::fmt::Debug for DlsFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DlsFont")
            .field("filename", &self.filename)
            .finish_non_exhaustive()
    }
}

impl Drop for DlsFont {
    /// `on_file_exit`: the file is closed with the font.
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            file.close();
        }
    }
}

/// Translation of `new_fluid_dls_font()`: the font, or `None` with the
/// exception logged.
fn new_fluid_dls_font(
    fcbs: &FluidFileCallbacks,
    filename: &str,
    output_sample_rate: u32,
    try_mlock: bool,
    default_mod_list: Option<Arc<Vec<FluidMod>>>,
) -> Option<DlsFont> {
    match DlsFont::new(
        fcbs,
        filename,
        output_sample_rate,
        try_mlock,
        default_mod_list,
    ) {
        Ok(dlsfont) => Some(dlsfont),
        Err(DlsException {
            kind: DlsExceptionKind::BadAlloc,
            ..
        }) => {
            flog!(FLUID_PANIC, "Out of memory when allocing fluid_dls_font");
            None
        }
        Err(exc) => {
            flog!(FLUID_ERR, "Exception thrown in fluid_dls_font constructor");
            log_exception(FLUID_ERR, &exc, 0);
            None
        }
    }
}

// (`delete_fluid_dls_font()` is `DlsFont`'s `Drop`, which can't throw.)

// Basic DLS structures

/// `FLUID_FOURCC()`.
const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from_le_bytes([a, b, c, d])
}

// non-LIST chunks
const RIFF_FCC: u32 = fourcc(b'R', b'I', b'F', b'F');
const LIST_FCC: u32 = fourcc(b'L', b'I', b'S', b'T');
const DLID_FCC: u32 = fourcc(b'd', b'l', b'i', b'd');
const CDL_FCC: u32 = fourcc(b'c', b'd', b'l', b' ');
const PTBL_FCC: u32 = fourcc(b'p', b't', b'b', b'l');
const VERS_FCC: u32 = fourcc(b'v', b'e', b'r', b's');
const COLH_FCC: u32 = fourcc(b'c', b'o', b'l', b'h');
const INSH_FCC: u32 = fourcc(b'i', b'n', b's', b'h');
const WSMP_FCC: u32 = fourcc(b'w', b's', b'm', b'p');
const FMT_FCC: u32 = fourcc(b'f', b'm', b't', b' ');
const DATA_FCC: u32 = fourcc(b'd', b'a', b't', b'a');
const ART1_FCC: u32 = fourcc(b'a', b'r', b't', b'1');
const ART2_FCC: u32 = fourcc(b'a', b'r', b't', b'2');
const RGNH_FCC: u32 = fourcc(b'r', b'g', b'n', b'h');
const WLNK_FCC: u32 = fourcc(b'w', b'l', b'n', b'k');

// LIST chunks
const INFO_FCC: u32 = fourcc(b'I', b'N', b'F', b'O');
const LINS_FCC: u32 = fourcc(b'l', b'i', b'n', b's');
const WVPL_FCC: u32 = fourcc(b'w', b'v', b'p', b'l');
const INS_FCC: u32 = fourcc(b'i', b'n', b's', b' ');
const WAVE_FCC: u32 = fourcc(b'w', b'a', b'v', b'e');
const LART_FCC: u32 = fourcc(b'l', b'a', b'r', b't');
const LRGN_FCC: u32 = fourcc(b'l', b'r', b'g', b'n');
const LAR2_FCC: u32 = fourcc(b'l', b'a', b'r', b'2');
const RGN_FCC: u32 = fourcc(b'r', b'g', b'n', b' ');
const RGN2_FCC: u32 = fourcc(b'r', b'g', b'n', b'2');

const DLS_FCC: u32 = fourcc(b'D', b'L', b'S', b' ');

// not required but recognized
const FACT_FCC: u32 = fourcc(b'f', b'a', b'c', b't');
const CUE_FCC: u32 = fourcc(b'c', b'u', b'e', b' ');

// this is seen as another 'dlid' in LIST[wave]
const GUID_FCC: u32 = fourcc(b'g', b'u', b'i', b'd');

// The 'crs1' chunk is seen in Crystal's DLS (The CrysDLS, see https://www.vogons.org/viewtopic.php?f=62&t=70307)
// It has wrong size and need manual fix
// See https://github.com/spessasus/spessasynth_core/issues/5 and https://github.com/FluidSynth/fluidsynth/pull/1626
const CRS1_FCC: u32 = fourcc(b'c', b'r', b's', b'1');

// The 'crs2' chunk is also seen in Crystal's DLS.
// But it has correct size (0 bytes!)
const CRS2_FCC: u32 = fourcc(b'c', b'r', b's', b'2');

// MobileBAE proprietary chunk for instrument aliasing
// https://lpcwiki.miraheze.org/wiki/MobileBAE#Proprietary_instrument_aliasing_chunk
const PGAL_FCC: u32 = fourcc(b'p', b'g', b'a', b'l');

// info
const INAM_FCC: u32 = fourcc(b'I', b'N', b'A', b'M');

/// `vector::reserve(n)`, whose allocation can fail.
fn try_reserve<T>(v: &mut Vec<T>, n: usize) -> DlsResult<()> {
    if n > v.capacity() {
        v.try_reserve_exact(n - v.len())?;
    }
    Ok(())
}

/// `vector::resize(n)` (or `string::resize(n)`), whose allocation can
/// fail.
fn try_resize<T: Clone + Default>(v: &mut Vec<T>, n: usize) -> DlsResult<()> {
    if n > v.len() {
        v.try_reserve_exact(n - v.len())?;
    }
    v.resize(n, T::default());
    Ok(())
}

impl DlsFont {
    /// `fcbs.fread()` of the font's file.
    fn fread(&mut self, buf: &mut [u8]) -> i32 {
        match self.file.as_mut() {
            Some(file) => file.read(buf),
            None => FLUID_FAILED,
        }
    }

    /// `fcbs.fseek()` of the font's file.
    fn fcbs_fseek(&mut self, pos: i64, whence: i32) -> i32 {
        match self.file.as_mut() {
            Some(file) => file.seek(pos, whence),
            None => FLUID_FAILED,
        }
    }

    /// `fcbs.ftell()` of the font's file.
    fn fcbs_ftell(&mut self) -> i64 {
        match self.file.as_mut() {
            Some(file) => file.tell(),
            None => FLUID_FAILED as i64,
        }
    }

    /// `fluid_dls_font::fseek()`.
    fn fseek(&mut self, pos: i64, whence: i32) -> DlsResult<()> {
        if self.fcbs_fseek(pos, whence) != FLUID_OK {
            return Err(DlsException::runtime("Failed to seek"));
        }
        Ok(())
    }

    /// `fluid_dls_font::fskip()`.
    fn fskip(&mut self, pos: i64) -> DlsResult<()> {
        self.fseek(pos, SEEK_CUR)
    }

    /// `READCHUNK_RAW()`.
    fn read_chunk_raw(&mut self) -> DlsResult<RiffChunk> {
        let mut buf = [0u8; 8];
        if self.fread(&mut buf) == FLUID_FAILED {
            return Err(DlsException::runtime("Failed when reading chunk"));
        }
        Ok(RiffChunk {
            id: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            size: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        })
    }

    /// `READID()`.
    fn read_id(&mut self) -> DlsResult<u32> {
        let mut buf = [0u8; 4];
        if self.fread(&mut buf) == FLUID_FAILED {
            return Err(DlsException::runtime("Failed when reading FOURCC ID"));
        }
        Ok(u32::from_le_bytes(buf))
    }

    /// `READ8()`.
    fn read8(&mut self) -> DlsResult<u8> {
        let mut buf = [0u8; 1];
        if self.fread(&mut buf) == FLUID_FAILED {
            return Err(DlsException::runtime("Failed when reading byte"));
        }
        Ok(buf[0])
    }

    /// `READ16()`.
    fn read16(&mut self) -> DlsResult<u16> {
        let mut buf = [0u8; 2];
        if self.fread(&mut buf) == FLUID_FAILED {
            return Err(DlsException::runtime("Failed when reading word"));
        }
        Ok(u16::from_le_bytes(buf))
    }

    /// `READ32()`.
    fn read32(&mut self) -> DlsResult<u32> {
        let mut buf = [0u8; 4];
        if self.fread(&mut buf) == FLUID_FAILED {
            return Err(DlsException::runtime("Failed when reading dword"));
        }
        Ok(u32::from_le_bytes(buf))
    }
}

/// Translation of `READCHUNK()`: reads a chunk's header, returning the
/// header size (a LIST's or the RIFF's is 12 bytes, its type its id).
fn read_chunk(dlsfont: &mut DlsFont, chunk: &mut RiffChunk) -> DlsResult<i32> {
    *chunk = dlsfont.read_chunk_raw()?;

    if chunk.id == LIST_FCC {
        if chunk.size < 4 {
            return Err(DlsException::runtime("Bad LIST header: size < 4"));
        }

        chunk.id = dlsfont.read_id()?;
        chunk.size -= 4;
        return Ok(12);
    }

    if chunk.id == RIFF_FCC {
        if chunk.size < 4 {
            return Err(DlsException::runtime("Bad RIFF header: size < 4"));
        }

        chunk.id = dlsfont.read_id()?;
        chunk.size -= 4;
        return Ok(12);
    }

    if chunk.id == CRS1_FCC {
        // see CRS1_FCC comment
        chunk.size = 28;
    }

    Ok(8)
}

impl DlsFont {
    /// Translation of `fluid_dls_font::visit_subchunks()`: calls
    /// `visitor(self, subchunk, headersize, pos)` for each subchunk of the
    /// LIST at `offset`.
    fn visit_subchunks<V>(
        &mut self,
        offset: i64,
        expected_4cc: u32,
        mut visitor: V,
    ) -> DlsResult<RiffChunk>
    where
        V: FnMut(&mut DlsFont, RiffChunk, i32, i64) -> DlsResult<()>,
    {
        self.fseek(offset, SEEK_SET)?;
        let mut chunk = RiffChunk::default();

        // only LIST chunks contains subchunks
        if read_chunk(self, &mut chunk)? != 12 {
            if expected_4cc == 0 {
                return Err(DlsException::runtime(format!(
                    "Expected chunk LIST[....], got {}",
                    fmt_4cc(chunk.id)
                )));
            }

            return Err(DlsException::runtime(format!(
                "Expected chunk LIST[{}], got {}",
                fmt_4cc(expected_4cc), // 0x20202020 is '    ' (four spaces)
                fmt_4cc(chunk.id)
            )));
        }

        if expected_4cc != 0 && chunk.id != expected_4cc {
            return Err(DlsException::runtime(format!(
                "Expected chunk LIST[{}], got LIST[{}]",
                fmt_4cc(expected_4cc),
                fmt_4cc(chunk.id)
            )));
        }

        let mut pos = offset + 12;

        while pos < offset + 12 + chunk.size as i64 {
            let mut subchunk = RiffChunk::default();
            self.fseek(pos, SEEK_SET)?;
            let subchunkpos = pos;
            let headersize = read_chunk(self, &mut subchunk)?;
            pos += headersize as i64;

            if let Err(exc) = visitor(self, subchunk, headersize, subchunkpos) {
                return Err(DlsException::runtime(format!(
                    "visit in LIST '{}' ofs=0x{:x} -> {}'{}' ofs=0x{:x}",
                    fmt_4cc(chunk.id),
                    offset as u64,
                    if headersize == 12 { "LIST " } else { "" },
                    fmt_4cc(subchunk.id),
                    subchunkpos as u64
                ))
                .nest(exc));
            }

            pos += subchunk.size as i64;

            if subchunk.size % 2 != 0 {
                pos += 1;
            }
        }

        Ok(chunk)
    }

    /// Translation of `fluid_dls_font::read_name_from_info_entries()`.
    fn read_name_from_info_entries(&mut self, offset: i64, size: u32) -> DlsResult<String> {
        self.fseek(offset, SEEK_SET)?;
        let mut pos = offset;

        while pos < offset + size as i64 {
            let mut chunk = RiffChunk::default();
            self.fseek(pos, SEEK_SET)?;
            pos += read_chunk(self, &mut chunk)? as i64;

            if chunk.id == INAM_FCC {
                let mut name = Vec::new();
                try_resize(&mut name, chunk.size as usize)?;

                if self.fread(&mut name) != FLUID_OK {
                    return Err(DlsException::runtime("Failed when reading INAM chunk"));
                }

                // (std::string holds the bytes: each as a char, the NULs too)
                let name: String = name.iter().map(|&b| b as char).collect();
                flog!(FLUID_DBG, "Read INAM: '{}'", c_str(&name));
                return Ok(name);
            }

            pos += chunk.size as i64;

            if chunk.size % 2 != 0 {
                pos += 1;
            }
        }

        Ok(String::new())
    }
}

/// Translation of `DLSID`: DLS-2.2 2.5 <dlid-ck>, DLSID Chunk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Dlsid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

/// `DEFINE_DLSID()`.
const fn dlsid(l: u32, w1: u16, w2: u16, b: [u8; 8]) -> Dlsid {
    Dlsid {
        data1: l,
        data2: w1,
        data3: w2,
        data4: b,
    }
}

// Specified in DLS-2.2 2.6 <cdl-ck>, Conditional Chunk
// From DLS-2.2 4.2 DLS Level 2 Header File (dls2.h)
// Copyright: Written by Microsoft 1998. Released for public use.
const DLSID_GM_IN_HARDWARE: Dlsid = dlsid(
    0x178f2f24,
    0xc364,
    0x11d1,
    [0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12],
);
const DLSID_GS_IN_HARDWARE: Dlsid = dlsid(
    0x178f2f25,
    0xc364,
    0x11d1,
    [0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12],
);
const DLSID_XG_IN_HARDWARE: Dlsid = dlsid(
    0x178f2f26,
    0xc364,
    0x11d1,
    [0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12],
);
const DLSID_SUPPORTS_DLS1: Dlsid = dlsid(
    0x178f2f27,
    0xc364,
    0x11d1,
    [0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12],
);
const DLSID_SUPPORTS_DLS2: Dlsid = dlsid(
    0xf14599e5,
    0x4689,
    0x11d2,
    [0xaf, 0xa6, 0x0, 0xaa, 0x0, 0x24, 0xd8, 0xb6],
);
const DLSID_SAMPLE_MEMORY_SIZE: Dlsid = dlsid(
    0x178f2f28,
    0xc364,
    0x11d1,
    [0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12],
);
const DLSID_MANUFACTURERS_ID: Dlsid = dlsid(
    0xb03e1181,
    0x8095,
    0x11d2,
    [0xa1, 0xef, 0x0, 0x60, 0x8, 0x33, 0xdb, 0xd8],
);
const DLSID_PRODUCT_ID: Dlsid = dlsid(
    0xb03e1182,
    0x8095,
    0x11d2,
    [0xa1, 0xef, 0x0, 0x60, 0x8, 0x33, 0xdb, 0xd8],
);
const DLSID_SAMPLE_PLAYBACK_RATE: Dlsid = dlsid(
    0x2a91f713,
    0xa4bf,
    0x11d2,
    [0xbb, 0xdf, 0x0, 0x60, 0x8, 0x33, 0xdb, 0xd8],
);

/// Translation of `READGUID()`.
fn read_guid(sf: &mut DlsFont, id: &mut Dlsid) -> DlsResult<()> {
    id.data1 = sf.read32()?;
    id.data2 = sf.read16()?;
    id.data3 = sf.read16()?;

    if sf.fread(&mut id.data4) == FLUID_FAILED {
        return Err(DlsException::runtime("Failed when reading GUID"));
    }
    Ok(())
}

// macro WAVE_FORMAT_PCM is defined in some Windows headers
const WAVE_FORMAT_PCM: u16 = 0x0001;

/// Translation of `read_data_lpcm()`: `size` bytes of `data` into `dest`.
fn read_data_lpcm(dest: &mut [i16], data: &[u8], size: usize, bitdepth: u16) -> DlsResult<()> {
    if bitdepth == 16 {
        // Note (upstream): memcpy() of the little-endian data, which is
        // wrong on big-endian hosts; the translation reads it as
        // little-endian everywhere.
        for (d, s) in dest.iter_mut().zip(data[..size].chunks_exact(2)) {
            *d = i16::from_le_bytes([s[0], s[1]]);
        }
        return Ok(());
    }

    if bitdepth != 8 {
        // this should never happen!
        return Err(DlsException::runtime("Unsupported bitdepth"));
    }

    for (d, &s) in dest.iter_mut().zip(&data[..size]) {
        *d = (s.wrapping_sub(128) as i8 as i16) << 8;
    }
    Ok(())
}

// DLS connection blocks (DLS-2.2 1.6.5 and 2.11 <art2-ck>)

// Generic Sources
const CONN_SRC_NONE: u16 = 0x0000; // No Source
const CONN_SRC_LFO: u16 = 0x0001; // Low Frequency Oscillator
const CONN_SRC_KEYONVELOCITY: u16 = 0x0002; // Note-On Velocity
const CONN_SRC_KEYNUMBER: u16 = 0x0003; // Note Number
const CONN_SRC_EG1: u16 = 0x0004; // Envelope Generator 1
const CONN_SRC_EG2: u16 = 0x0005; // Envelope Generator 2
const CONN_SRC_PITCHWHEEL: u16 = 0x0006; // Pitch Wheel
const CONN_SRC_POLYPRESSURE: u16 = 0x0007; // Polyphonic Pressure
const CONN_SRC_CHANNELPRESSURE: u16 = 0x0008; // Channel Pressure
const CONN_SRC_VIBRATO: u16 = 0x0009; // Vibrato LFO

// Midi Controllers 0-127
const CONN_SRC_CC1: u16 = 0x0081; // Modulation
const CONN_SRC_CC7: u16 = 0x0087; // Channel Volume
const CONN_SRC_CC10: u16 = 0x008a; // Pan
const CONN_SRC_CC11: u16 = 0x008b; // Expression
const CONN_SRC_CC91: u16 = 0x00db; // Chorus Send
const CONN_SRC_CC93: u16 = 0x00dd; // Reverb Send

// Registered Parameter Numbers
const CONN_SRC_RPN0: u16 = 0x0100; // RPN0 - Pitch Bend Range
const CONN_SRC_RPN1: u16 = 0x0101; // RPN1 - Fine Tune
const CONN_SRC_RPN2: u16 = 0x0102; // RPN2 - Coarse Tune

// Generic Destinations
const CONN_DST_NONE: u16 = 0x0000; // No Destination
const CONN_DST_GAIN: u16 = 0x0001; // Gain
const CONN_DST_RESERVED: u16 = 0x0002; // Reserved
const CONN_DST_PITCH: u16 = 0x0003; // Pitch
const CONN_DST_PAN: u16 = 0x0004; // Pan
const CONN_DST_KEYNUMBER: u16 = 0x0005; // Key Number Generator

// Channel Output Destinations
const CONN_DST_LEFT: u16 = 0x0010; // Left Channel Send
const CONN_DST_RIGHTREAR: u16 = 0x0015; // Right Rear Channel Send
const CONN_DST_CHORUS: u16 = 0x0080; // Chorus Send
const CONN_DST_REVERB: u16 = 0x0081; // Reverb Send

// Modulator LFO Destinations
const CONN_DST_LFO_FREQUENCY: u16 = 0x0104; // LFO Frequency
const CONN_DST_LFO_STARTDELAY: u16 = 0x0105; // LFO Start Delay Time

// Vibrato LFO Destinations
const CONN_DST_VIB_FREQUENCY: u16 = 0x0114; // Vibrato Frequency
const CONN_DST_VIB_STARTDELAY: u16 = 0x0115; // Vibrato Start Delay

// EG Destinations
const CONN_DST_EG1_ATTACKTIME: u16 = 0x0206; // EG1 Attack Time
const CONN_DST_EG1_DECAYTIME: u16 = 0x0207; // EG1 Decay Time
const CONN_DST_EG1_RESERVED: u16 = 0x0208; // EG1 Reserved
const CONN_DST_EG1_RELEASETIME: u16 = 0x0209; // EG1 Release Time
const CONN_DST_EG1_SUSTAINLEVEL: u16 = 0x020A; // EG1 Sustain Level
const CONN_DST_EG1_DELAYTIME: u16 = 0x020B; // EG1 Delay Time
const CONN_DST_EG1_HOLDTIME: u16 = 0x020C; // EG1 Hold Time
const CONN_DST_EG1_SHUTDOWNTIME: u16 = 0x020D; // EG1 Shutdown Time

const CONN_DST_EG2_ATTACKTIME: u16 = 0x030A; // EG2 Attack Time
const CONN_DST_EG2_DECAYTIME: u16 = 0x030B; // EG2 Decay Time
const CONN_DST_EG2_RESERVED: u16 = 0x030C; // EG2 Reserved
const CONN_DST_EG2_RELEASETIME: u16 = 0x030D; // EG2 Release Time
const CONN_DST_EG2_SUSTAINLEVEL: u16 = 0x030E; // EG2 Sustain Level
const CONN_DST_EG2_DELAYTIME: u16 = 0x030F; // EG2 Delay Time
const CONN_DST_EG2_HOLDTIME: u16 = 0x0310; // EG2 Hold Time

// Filter Destinations
const CONN_DST_FILTER_CUTOFF: u16 = 0x0500; // Filter Cutoff Frequency
const CONN_DST_FILTER_Q: u16 = 0x0501; // Filter Resonance

// Transforms
const CONN_TRN_NONE: u16 = 0x0000; // No Transform
const CONN_TRN_CONCAVE: u16 = 0x0001; // Concave Transform
const CONN_TRN_CONVEX: u16 = 0x0002; // Convex Transform
const CONN_TRN_SWITCH: u16 = 0x0003; // Switch Transform

// Transform masks
const TRN_SRC_INV_MASK: u16 = 0b1000_0000_0000_0000;
const TRN_SRC_BIP_MASK: u16 = 0b0100_0000_0000_0000;
const TRN_SRC_TRN_MASK: u16 = 0b0011_1100_0000_0000;
const TRN_CTL_INV_MASK: u16 = 0b0000_0010_0000_0000;
const TRN_CTL_BIP_MASK: u16 = 0b0000_0001_0000_0000;
const TRN_CTL_TRN_MASK: u16 = 0b0000_0000_1111_0000;
const TRN_OUT_TRN_MASK: u16 = 0b0000_0000_0000_1111;

/// Translation of `DLSTransform`: the fields of a connection block's
/// usTransform (each transform 4 bits wide).
#[derive(Clone, Copy, Debug)]
struct DlsTransform {
    out_trans: u16,
    ctl_trans: u16,
    src_trans: u16,
    ctl_inv: bool,
    src_inv: bool,
    ctl_bip: bool,
    src_bip: bool,
}

impl DlsTransform {
    fn new(transform: u16) -> DlsTransform {
        DlsTransform {
            out_trans: transform & TRN_OUT_TRN_MASK,
            ctl_trans: (transform & TRN_CTL_TRN_MASK) >> 4,
            src_trans: (transform & TRN_SRC_TRN_MASK) >> 10,
            ctl_inv: (transform & TRN_CTL_INV_MASK) != 0,
            src_inv: (transform & TRN_SRC_INV_MASK) != 0,
            ctl_bip: (transform & TRN_CTL_BIP_MASK) != 0,
            src_bip: (transform & TRN_SRC_BIP_MASK) != 0,
        }
    }

    /// `operator uint16_t()`.
    fn to_u16(self) -> u16 {
        (if self.src_inv { TRN_SRC_INV_MASK } else { 0 })
            | (if self.src_bip { TRN_SRC_BIP_MASK } else { 0 })
            | (self.src_trans << 10)
            | (if self.ctl_inv { TRN_CTL_INV_MASK } else { 0 })
            | (if self.ctl_bip { TRN_CTL_BIP_MASK } else { 0 })
            | (self.ctl_trans << 4)
            | self.out_trans
    }
}

/// Translation of `dls_transform_ctl_to_src()`: move usControl transform
/// to usSource transform.
const fn dls_transform_ctl_to_src(transform: u16) -> u16 {
    ((transform << 6) & 0b1111_1100_0000_0000) | (transform & 0b1111)
}

/// Translation of `convert_dls_transform_to_fluid()`.
const fn convert_dls_transform_to_fluid(transform_mode: u16) -> Option<u8> {
    match transform_mode {
        CONN_TRN_NONE => Some(FLUID_MOD_LINEAR),
        CONN_TRN_CONCAVE => Some(FLUID_MOD_CONCAVE),
        CONN_TRN_CONVEX => Some(FLUID_MOD_CONVEX),
        CONN_TRN_SWITCH => Some(FLUID_MOD_SWITCH),
        _ => None,
    }
}

/// Translation of `convert_dls_mod_dest_to_gen()`.
const fn convert_dls_mod_dest_to_gen(dest: u16) -> Option<u16> {
    if dest == CONN_DST_NONE
        || dest == CONN_DST_RESERVED
        || dest == CONN_DST_EG1_RESERVED
        || dest == CONN_DST_EG2_RESERVED
        || (dest >= CONN_DST_LEFT && dest <= CONN_DST_RIGHTREAR)
    {
        return None;
    }

    let gen = match dest {
        CONN_DST_GAIN => GEN_ATTENUATION, // NOTE: attenuation = -gain
        CONN_DST_EG1_SUSTAINLEVEL => GEN_VOLENVSUSTAIN, // NOTE: SF2 sustain (atten) in dB = (100% - sustainlevel) * 96 dB (or 144 dB, whatever)
        CONN_DST_EG2_SUSTAINLEVEL => GEN_MODENVSUSTAIN, // NOTE: SF2 sustain (atten) = 100% - sustainlevel
        CONN_DST_PITCH => GEN_FINETUNE,
        CONN_DST_PAN => GEN_PAN,
        CONN_DST_CHORUS => GEN_CHORUSSEND,
        CONN_DST_REVERB => GEN_REVERBSEND,
        CONN_DST_LFO_FREQUENCY => GEN_MODLFOFREQ,
        CONN_DST_LFO_STARTDELAY => GEN_MODLFODELAY,
        CONN_DST_VIB_FREQUENCY => GEN_VIBLFOFREQ,
        CONN_DST_VIB_STARTDELAY => GEN_VIBLFODELAY,
        CONN_DST_EG1_ATTACKTIME => GEN_VOLENVATTACK,
        CONN_DST_EG1_DECAYTIME => GEN_VOLENVDECAY,
        CONN_DST_EG1_RELEASETIME => GEN_VOLENVRELEASE,
        CONN_DST_EG1_DELAYTIME => GEN_VOLENVDELAY,
        CONN_DST_EG1_HOLDTIME => GEN_VOLENVHOLD,
        CONN_DST_EG1_SHUTDOWNTIME => return None,
        CONN_DST_EG2_ATTACKTIME => GEN_MODENVATTACK,
        CONN_DST_EG2_DECAYTIME => GEN_MODENVDECAY,
        CONN_DST_EG2_RELEASETIME => GEN_MODENVRELEASE,
        CONN_DST_EG2_DELAYTIME => GEN_MODENVDELAY,
        CONN_DST_EG2_HOLDTIME => GEN_MODENVHOLD,
        CONN_DST_FILTER_CUTOFF => GEN_FILTERFC,
        CONN_DST_FILTER_Q => GEN_FILTERQ,
        _ => return None,
    };
    Some(gen as u16)
}

/// Translation of `convert_dls_mod_gsrc()`: gsrc: general source.
const fn convert_dls_mod_gsrc(source: u16) -> Option<u16> {
    let src = match source {
        CONN_SRC_NONE => FLUID_MOD_NONE,
        CONN_SRC_KEYONVELOCITY => FLUID_MOD_VELOCITY,
        CONN_SRC_KEYNUMBER => FLUID_MOD_KEY,
        CONN_SRC_POLYPRESSURE => FLUID_MOD_KEYPRESSURE,
        CONN_SRC_CHANNELPRESSURE => FLUID_MOD_CHANNELPRESSURE,
        CONN_SRC_PITCHWHEEL => FLUID_MOD_PITCHWHEEL,
        CONN_SRC_RPN0 => FLUID_MOD_PITCHWHEELSENS,
        _ => return None,
    };
    Some(src as u16)
}

/// Translation of `convert_dls_mod_csrc()`: csrc: CC source.
const fn convert_dls_mod_csrc(source: u16) -> Option<u16> {
    match source {
        CONN_SRC_CC1 => Some(1),
        CONN_SRC_CC7 => Some(7),
        CONN_SRC_CC10 => Some(10),
        CONN_SRC_CC11 => Some(11),
        CONN_SRC_CC91 => Some(91),
        CONN_SRC_CC93 => Some(93),
        _ => None,
    }
}

/// Translation of `add_dls_connectionblock_to_art()` (with the control
/// source; the overload without one passes `CONN_SRC_NONE`).
fn add_dls_connectionblock_to_art(
    art: &mut DlsArticulation,
    dest_gen: u16,
    src_dls: u16,
    scale: FluidReal,
    mut transform: DlsTransform,
    control_dls: u16,
) {
    if src_dls == CONN_SRC_NONE && control_dls == CONN_SRC_NONE {
        let gen = &mut art.gens[dest_gen as usize];
        *gen = Some(gen.unwrap_or(0.0) + scale);
        return;
    }

    // DLS-2 1.6.5.5 Output Transforms
    // There are no output transforms defined in the DLS Level 2. The bits in the output transform
    // field must all be set to zero for compatibility with future versions of DLS.

    // if output transform is set to something else, but input transform is none, and the connection
    // block does not have a control source, apply the output transform to the input instead.
    if transform.out_trans != CONN_TRN_NONE
        && transform.src_trans == CONN_TRN_NONE
        && control_dls == CONN_SRC_NONE
    {
        transform.src_trans = transform.out_trans;
        transform.out_trans = CONN_TRN_NONE;
    }

    if transform.out_trans != CONN_TRN_NONE {
        flog!(
            FLUID_WARN,
            "Output transform in connection block is not supported, set to to linear"
        );
    }

    let mut mod_ = FluidMod::default();
    let mut src = convert_dls_mod_gsrc(src_dls);

    if src.is_some() {
        mod_.flags1 = FLUID_MOD_GC;
    } else {
        src = convert_dls_mod_csrc(src_dls);

        if src.is_some() {
            mod_.flags1 = FLUID_MOD_CC;
        } else {
            flog!(
                FLUID_WARN,
                "Ignoring connection block with unsupported source for srcOper: {}",
                src_dls
            );
            return;
        }
    }

    mod_.src1 = src.unwrap_or_default() as u8;

    let mut ctl = convert_dls_mod_gsrc(control_dls);

    if ctl.is_some() {
        mod_.flags2 = FLUID_MOD_GC;
    } else {
        ctl = convert_dls_mod_csrc(control_dls);

        if ctl.is_some() {
            mod_.flags2 = FLUID_MOD_CC;
        } else {
            flog!(
                FLUID_WARN,
                "Ignoring connection block with unsupported source for amtSrcOper: {}",
                control_dls
            );
            return;
        }
    }

    mod_.src2 = ctl.unwrap_or_default() as u8;

    if mod_.src1 != FLUID_MOD_NONE {
        mod_.flags1 |= if transform.src_bip {
            FLUID_MOD_BIPOLAR
        } else {
            FLUID_MOD_UNIPOLAR
        };
        mod_.flags1 |= if transform.src_inv {
            FLUID_MOD_NEGATIVE
        } else {
            FLUID_MOD_POSITIVE
        };
        let Some(transform_flag) = convert_dls_transform_to_fluid(transform.src_trans) else {
            flog!(
                FLUID_WARN,
                "Invalid DLS transform enum, ignoring connection block"
            );
            return;
        };

        mod_.flags1 |= transform_flag;
    }

    if mod_.src2 != FLUID_MOD_NONE {
        mod_.flags2 |= if transform.ctl_bip {
            FLUID_MOD_BIPOLAR
        } else {
            FLUID_MOD_UNIPOLAR
        };
        mod_.flags2 |= if transform.ctl_inv {
            FLUID_MOD_NEGATIVE
        } else {
            FLUID_MOD_POSITIVE
        };
        let Some(transform_flag) = convert_dls_transform_to_fluid(transform.ctl_trans) else {
            flog!(
                FLUID_WARN,
                "Invalid DLS transform enum, ignoring connection block"
            );
            return;
        };

        mod_.flags2 |= transform_flag;
    }

    mod_.amount = scale;
    mod_.dest = dest_gen as u8;

    // transform.out_trans is ignored because unsupported
    mod_.trans = FLUID_MOD_TRANSFORM_LINEAR;

    if mod_.src1 == FLUID_MOD_VELOCITY
        && mod_.dest as usize == GEN_ATTENUATION
        && (mod_.flags1 & FLUID_MOD_CONCAVE) != 0
    {
        // confirmed as vel2att modulator
        // fix when non-inverted (positive), this is seen in Fury.dls from https://www.ronimusic.com/smp_ios_dls_files.htm
        if (mod_.flags1 & FLUID_MOD_NEGATIVE) == 0 {
            flog!(
                FLUID_DBG,
                "Fixing non-inverted vel2att modulator to inverted"
            );
            mod_.flags1 |= FLUID_MOD_NEGATIVE;
        }
    }

    art.add_mod(&mod_);
}

/// Translation of `convert_dls_connectionblock_to_art()`.
fn convert_dls_connectionblock_to_art(
    art: &mut DlsArticulation,
    mut source: u16,
    mut control: u16,
    destination: u16,
    mut transform: u16,
    scale_16: i32,
) {
    match destination {
        CONN_DST_NONE
        // these reserved dest are used in gm.dls, interesting
        | CONN_DST_RESERVED
        | CONN_DST_EG1_RESERVED
        | CONN_DST_EG2_RESERVED => return,

        _ => {}
    }

    if source == CONN_SRC_NONE && control != CONN_SRC_NONE {
        source = std::mem::replace(&mut control, CONN_SRC_NONE);
        transform = dls_transform_ctl_to_src(transform);
    }

    if source == CONN_SRC_RPN1 {
        flog!(FLUID_WARN, "Ignoring connection block with RPN 1 source");
        return;
    }

    if source == CONN_SRC_RPN2 {
        flog!(FLUID_WARN, "Ignoring connection block with RPN 2 source");
        return;
    }

    if source == CONN_SRC_EG1 {
        flog!(FLUID_WARN, "Ignoring connection block with EG1 source");
        return;
    }

    let trans = DlsTransform::new(transform);
    let mut scale = scale_16 as FluidReal / 65536.0;

    // keynum * scale -> keynum, linear
    if source == CONN_SRC_KEYNUMBER
        && control == CONN_SRC_NONE
        && destination == CONN_DST_KEYNUMBER
        && transform == 0
    {
        art.keynum_scale = scale / 12800.0;
        return;
    }

    // (the LFO, vibrato and EG2 connections that become generators, as the
    // control's transform moved to the source's)
    let to_gen = |art: &mut DlsArticulation, gen: usize| {
        add_dls_connectionblock_to_art(
            art,
            gen as u16,
            control,
            scale,
            DlsTransform::new(dls_transform_ctl_to_src(transform)),
            CONN_SRC_NONE,
        );
    };

    // LFO or EG -> something
    // they are not available as modulator source in SF2
    // note: keynumToVol/ModEnvHold/Decay is implemented through modulator. why they exist?
    if source == CONN_SRC_LFO {
        if !trans.src_bip {
            flog!(
                FLUID_DBG,
                "Non bipolar LFO source is not supported, set to bipolar"
            );
        }

        if trans.src_inv || trans.src_trans != CONN_TRN_NONE {
            flog!(
                FLUID_WARN,
                "LFO source is inverted or transformed, ignoring transform"
            );
        }

        match destination {
            CONN_DST_PITCH => {
                to_gen(art, GEN_MODLFOTOPITCH);
                return;
            }

            CONN_DST_FILTER_CUTOFF => {
                to_gen(art, GEN_MODLFOTOFILTERFC);
                return;
            }

            CONN_DST_GAIN => {
                to_gen(art, GEN_MODLFOTOVOL);
                return;
            }

            _ => {}
        }
    } else if source == CONN_SRC_VIBRATO {
        if !trans.src_bip {
            flog!(
                FLUID_DBG,
                "Non bipolar vibrato LFO source is not supported, set to bipolar"
            );
        }

        if trans.src_inv || trans.src_trans != CONN_TRN_NONE {
            flog!(
                FLUID_WARN,
                "Inverted or transformed vibrato LFO source is not supported, ignoring transform"
            );
        }

        if destination == CONN_DST_PITCH {
            to_gen(art, GEN_VIBLFOTOPITCH);
            return;
        }
    } else if source == CONN_SRC_EG2 {
        if trans.src_inv || trans.src_bip || trans.src_trans != CONN_TRN_NONE {
            flog!(FLUID_WARN, "EG2 source is transformed, ignoring transform");
        }

        match destination {
            CONN_DST_PITCH => {
                to_gen(art, GEN_MODENVTOPITCH);
                return;
            }

            CONN_DST_FILTER_CUTOFF => {
                to_gen(art, GEN_MODENVTOFILTERFC);
                return;
            }

            _ => {}
        }
    }

    if destination == CONN_DST_GAIN {
        scale = -scale;
    }

    if destination == CONN_DST_EG1_SUSTAINLEVEL {
        let gen = &mut art.gens[GEN_VOLENVSUSTAIN];
        *gen = Some(gen.unwrap_or(0.0) + 960.0);
        scale = -scale * 960.0 / 1000.0;
    }

    if destination == CONN_DST_EG2_SUSTAINLEVEL {
        let gen = &mut art.gens[GEN_MODENVSUSTAIN];
        *gen = Some(gen.unwrap_or(0.0) + 1000.0);
        scale = -scale;
    }

    let Some(dest_gen) = convert_dls_mod_dest_to_gen(destination) else {
        flog!(
            FLUID_DBG,
            "Ignoring connection block with unsupported destination: {}",
            destination
        );
        return;
    };

    add_dls_connectionblock_to_art(art, dest_gen, source, scale, trans, control);
}

impl DlsFont {
    /// Translation of the `fluid_dls_font` constructor: parse the DLS
    /// structure.
    fn new(
        fcbs: &FluidFileCallbacks,
        filename: &str,
        output_sample_rate: u32,
        try_mlock: bool,
        default_mod_list: Option<Arc<Vec<FluidMod>>>,
    ) -> DlsResult<DlsFont> {
        // Get basic file information

        let Some(file) = (fcbs.fopen)(filename) else {
            return Err(DlsException::runtime(format!(
                "Unable to open file '{}'",
                filename
            )));
        };

        let mut dlsfont = DlsFont {
            output_sample_rate,
            filename: filename.to_owned(),
            default_mod_list,
            file: Some(file),
            filesize: 0,
            linsoffset: 0,
            wvploffset: 0,
            pgaloffset: 0,
            pgalsize: 0,
            sampledata: Vec::new(),
            poolcues: Vec::new(),
            samples: Vec::new(),
            articulations: Vec::new(),
            instruments: Vec::new(),
            presets: Vec::new(),
            samples_fluid: Arc::default(),
            drum_note_aliasing: None,
            fluid_preset_iterator: 0,
            total_presets: 0,
        };
        dlsfont.parse(try_mlock)?;
        Ok(dlsfont)
    }

    /// The constructor's body, after the file is open.
    fn parse(&mut self, try_mlock: bool) -> DlsResult<()> {
        if self.fcbs_fseek(0, SEEK_END) == FLUID_FAILED {
            return Err(DlsException::runtime("Seek to end of file failed"));
        }

        self.filesize = self.fcbs_ftell();

        if self.filesize == FLUID_FAILED as i64 {
            return Err(DlsException::runtime("Get end of file position failed"));
        }

        if self.fcbs_fseek(0, SEEK_SET) == FLUID_FAILED {
            return Err(DlsException::runtime("Rewind to start of file failed"));
        }

        if self.filesize < 12 {
            return Err(DlsException::runtime("File too short! (< 12 bytes)"));
        }

        // Parse DLS
        // chunk: RIFF[DLS ]
        // subchunk: ...

        let mut chunk = self.read_chunk_raw()?; // load RIFF chunk

        if chunk.id != RIFF_FCC {
            return Err(DlsException::runtime("Not a RIFF file"));
        }

        chunk.id = self.read_id()?; // load file ID

        if chunk.id != DLS_FCC {
            return Err(DlsException::runtime(format!(
                "Not a DLS file. Expected header identifier 'DLS ' but found '{}'",
                fmt_4cc(chunk.id)
            )));
        }

        if chunk.size as i64 + 8 > self.filesize {
            let from = chunk.size.to_le_bytes();
            let max_size = self.filesize - 8;
            let to = (max_size as u32).to_le_bytes();
            let hint = format!(
                " (Hint: Try changing the hexadecimal little-endian size of the RIFF chunk from \
                 {:02X}{:02X}{:02X}{:02X} to {:02X}{:02X}{:02X}{:02X} using a hex editor.)",
                from[0], from[1], from[2], from[3], to[0], to[1], to[2], to[3]
            );
            return Err(DlsException::runtime(format!(
                "DLS file is too short! Outermost RIFF chunk reports a payload size of {} bytes, \
                 but there are only {} bytes left in the file.{}",
                chunk.size,
                max_size,
                if chunk.size as i64 == self.filesize {
                    hint.as_str()
                } else {
                    ""
                }
            )));
        }

        if (chunk.size as i64) + 8 < self.filesize {
            flog!(
                FLUID_WARN,
                "DLS file has extra data after RIFF chunk, ignoring."
            );
        }

        // we don't care about real file size after this point
        self.filesize = chunk.size as i64 + 8;

        // iterate over chunks in the RIFF form
        if let Err(exc) = self.visit_subchunks(0, DLS_FCC, |this, subchunk, headersize, pos| {
            match subchunk.id {
                // toplevel chunk
                DLID_FCC | VERS_FCC => {}

                CDL_FCC => {
                    // FIXME (upstream): the chunk size (a uint32_t) is
                    // passed as an int; above INT_MAX, no operation runs and
                    // the result is a "CDL stack underflow".
                    if !this.execute_cdls(pos + headersize as i64, subchunk.size as i32)? {
                        return Err(DlsException::runtime(
                            "DLS toplevel CDL bypasses the sound library",
                        ));
                    }
                }

                COLH_FCC => {
                    // read it now to preserve the instrument vector
                    if subchunk.size != 4 {
                        return Err(DlsException::runtime("DLS colh chunk size is not 4 bytes"));
                    }

                    let colh = this.read32()?;
                    try_reserve(&mut this.instruments, colh as usize)?;
                }

                PTBL_FCC => {
                    // read ptbl now
                    let cbsize = this.read32()?;

                    if cbsize < 8 {
                        return Err(DlsException::runtime("DLS ptbl chunk has invalid cbSize"));
                    }

                    let cues = this.read32()?; // sample count

                    // subchunk.size is uint32 and can be at max 0xFFFF'FFFF, therefore the max
                    // value of cues will be 0x3FFF'FFFF, as bigger values would require an uint64
                    // chunksize, contrary to the RIFF spec. Catch possible overflow here.
                    if cues > (u32::MAX - cbsize) / 4 {
                        return Err(DlsException::runtime(
                            "Too many poolcue records are contained in the ptbl chunk.",
                        ));
                    }

                    if cues * 4 + cbsize != subchunk.size {
                        return Err(DlsException::runtime("DLS ptbl chunk has corrupted size"));
                    }

                    this.fskip((cbsize - 8) as i64)?; // usually cbsize == 8

                    // FIXME (upstream): the size is only checked against the
                    // chunk's, not the file's: a small file can claim up to
                    // 0x3FFFFFFF cues, which are allocated (and zeroed)
                    // before they are read.
                    try_resize(&mut this.poolcues, cues as usize)?;

                    for i in 0..cues as usize {
                        this.poolcues[i] = this.read32()?;
                    }

                    try_reserve(&mut this.samples, cues as usize)?;
                }

                INFO_FCC => {
                    this.read_name_from_info_entries(pos + headersize as i64, subchunk.size)?;
                }

                LINS_FCC => {
                    this.linsoffset = pos;
                }

                WVPL_FCC => {
                    this.wvploffset = pos;
                }

                PGAL_FCC => {
                    this.pgaloffset = pos + headersize as i64;
                    this.pgalsize = subchunk.size as i64;
                }

                _ => {
                    flog!(
                        FLUID_WARN,
                        "{}",
                        c_str(&format!(
                            "Ignoring unknown top-level DLS chunk {}'{}' ofs=0x{:x}",
                            if headersize == 12 { "LIST " } else { "" },
                            fmt_4cc(subchunk.id),
                            pos as u64
                        ))
                    );
                }
            }
            Ok(())
        }) {
            return Err(
                DlsException::runtime("Exception thrown while reading RIFF form").nest(exc),
            );
        }

        // Parse samples (LIST[wvpl])
        let result = if self.wvploffset == 0 {
            Err(DlsException::runtime(
                "DLS does not contain a LIST[wvpl] chunk",
            ))
        } else {
            self.parse_wvpl(self.wvploffset)
        };
        if let Err(exc) = result {
            return Err(DlsException::runtime("Exception thrown while parsing samples").nest(exc));
        }

        // reading sample data is now completed

        flog!(
            FLUID_DBG,
            "DLS {} samples read, {} bytes",
            self.samples.len(),
            self.sampledata.len()
        );

        // Note: pinning the sample data to RAM (mlock_guard, for
        // synth.lock-memory) is left out, as the sample cache leaves it out
        // (see samplecache.rs). Where it fails, upstream logs "Failed to pin
        // the sample data to RAM; swapping is possible." (a warning) and goes
        // on; this is as upstream's builds without mlock (fluid_mlock() 0,
        // for example on Windows).
        let _ = try_mlock;

        // Parse LIST[lins]
        let result = if self.linsoffset == 0 {
            Err(DlsException::runtime(
                "DLS does not contain a LIST[lins] chunk",
            ))
        } else {
            self.parse_lins(self.linsoffset)
        };
        if let Err(exc) = result {
            return Err(
                DlsException::runtime("Exception thrown while parsing instruments").nest(exc),
            );
        }

        flog!(FLUID_DBG, "DLS {} instruments read", self.instruments.len());

        // Parse 'pgal'
        self.total_presets = self.instruments.len() as i64;

        if self.pgalsize != 0 {
            // (the size, a uint32_t, is passed as an int)
            if let Err(exc) = self.parse_pgal(self.pgaloffset, self.pgalsize as i32) {
                return Err(DlsException::runtime("Exception thrown while parsing pgal").nest(exc));
            }
        }

        // convert dls samples to fluid samples

        let mut samples_fluid: Vec<Arc<Sample>> = Vec::new();

        if let Err(exc) = try_reserve(&mut samples_fluid, self.samples.len()) {
            return Err(
                DlsException::runtime("Exception thrown while allocating fluid_sample_t").nest(exc),
            );
        }

        // (the sample data, which the samples share)
        let sampledata: Arc<[i16]> = Arc::from(std::mem::take(&mut self.sampledata));
        let buffer_size = (sampledata.len() * std::mem::size_of::<i16>()) as u32;
        let mut invalid_loops_were_sanitized = false;

        for sample in &self.samples {
            let mut fluid = Sample {
                start: sample.start,
                end: sample.end.wrapping_sub(1),
                samplerate: sample.samplerate,
                // (strncpy() of the name's C string into the 21 bytes of
                // fluid_sample_t's)
                name: c_str(&sample.name).chars().take(20).collect(),
                ..Sample::default()
            };

            if let Some(wsmp) = &sample.wsmp {
                fluid.loopstart = sample.start.wrapping_add(wsmp.loop_start);
                fluid.loopend = sample
                    .start
                    .wrapping_add(wsmp.loop_start)
                    .wrapping_add(wsmp.loop_length);
                fluid.origpitch = wsmp.unity_note as i32;
                fluid.pitchadj = wsmp.fine_tune as i32;
                invalid_loops_were_sanitized |= fluid_sample_sanitize_loop(&mut fluid, buffer_size);
            } else {
                // FIXME (upstream): a sample without a wsmp chunk gets the
                // original pitch 0, where DLS's default unity note is 60; a
                // region without a wsmp chunk of its own then plays it 60
                // semitones high.
                fluid.loopstart = 0;
                fluid.loopend = 0;
                fluid.origpitch = 0;
                fluid.pitchadj = 0;
            }

            fluid.data = sampledata.clone();
            fluid.sampletype = FLUID_SAMPLETYPE_MONO;
            fluid.default_modulators = self.default_mod_list.clone();

            if fluid_sample_validate(&mut fluid, buffer_size) != FLUID_OK {
                fluid = Sample::default();
            }

            samples_fluid.push(Arc::new(fluid));
        }

        if invalid_loops_were_sanitized {
            flog!(
                FLUID_WARN,
                "Some invalid DLS sample loops were sanitized! If you experience audible glitches, \
                 start fluidsynth in verbose mode for detailed information."
            );
        }

        // put info in dls_sample into region
        for instrument in &mut self.instruments {
            for region in &mut instrument.regions {
                // (samples.at(): out of range for a region without a wave
                // link, when there are no samples)
                let Some(sample) = self.samples.get(region.sampleindex as usize) else {
                    return Err(DlsException::out_of_range(
                        region.sampleindex as usize,
                        self.samples.len(),
                    ));
                };

                if let Some(wsmp) = &sample.wsmp {
                    if wsmp.loop_length == 0 {
                        region.samplemode_inherited = 0;
                    } else if wsmp.loop_type == 0 {
                        region.samplemode_inherited = 1;
                    } else if wsmp.loop_type == 1 {
                        region.samplemode_inherited = 3;
                    } else {
                        flog!(
                            FLUID_WARN,
                            "Invalid loop type {}, defaulting to loop and release",
                            wsmp.loop_type
                        );
                        region.samplemode_inherited = 3;
                    }

                    region.gain_inherited = wsmp.gain as FluidReal / 65536.0;
                } else {
                    region.samplemode_inherited = 0;
                    region.gain_inherited = 0.0;
                }
            }
        }

        // now samples list can be cleared
        self.samples = Vec::new();

        // Note (upstream): std::sort() is not stable, and its key leaves out
        // the drum flag, so the order of instruments with the same bank and
        // program (get_preset() returns the first that matches) is the C++
        // library's: libstdc++ keeps the file's order up to 16 instruments,
        // not above. This sort is stable: it keeps the file's order.
        self.instruments.sort_by(|lhs, rhs| {
            let l = lhs.bankmsb * 128 + lhs.banklsb;
            let r = rhs.bankmsb * 128 + rhs.banklsb;
            l.cmp(&r).then(lhs.pcnum.cmp(&rhs.pcnum))
        });

        // Note (upstream): the presets (instruments_fluid_data) are sorted
        // here as the instruments are, but before any is made: they stay in
        // the instruments' order, each instrument's aliases after it.

        // convert instruments to fluid presets
        self.samples_fluid = Arc::from(samples_fluid);
        let articulations: Arc<[DlsArticulation]> =
            Arc::from(std::mem::take(&mut self.articulations));
        let drum_note_aliasing = self.drum_note_aliasing.map(Arc::new);
        try_reserve(&mut self.presets, self.total_presets as usize)?;

        for mut instrument in std::mem::take(&mut self.instruments) {
            instrument.samples_fluid = self.samples_fluid.clone();
            instrument.articulations = articulations.clone();
            instrument.drum_note_aliasing = drum_note_aliasing.clone();

            // instrument.aliases is not used anymore, free it
            let aliases = std::mem::take(&mut instrument.aliases);
            // (the presets hold the instrument)
            let instrument = Arc::new(instrument);

            self.presets.push(Arc::new(DlsPreset {
                banklsb: instrument.banklsb,
                bankmsb: instrument.bankmsb,
                pcnum: instrument.pcnum,
                is_drums: instrument.is_drums,
                instrument: instrument.clone(),
            }));

            for alias in aliases {
                self.presets.push(Arc::new(DlsPreset {
                    banklsb: alias.banklsb,
                    bankmsb: alias.bankmsb,
                    pcnum: alias.pcnum,
                    is_drums: false,
                    instrument: instrument.clone(),
                }));
            }
        }

        Ok(())
    }

    // cdl

    /// Translation of `fluid_dls_font::eval_dlsid_query()`.
    fn eval_dlsid_query(&self, dlsid: &Dlsid) -> Option<u32> {
        if *dlsid == DLSID_GM_IN_HARDWARE {
            return Some(1);
        }

        if *dlsid == DLSID_GS_IN_HARDWARE {
            return Some(1);
        }

        if *dlsid == DLSID_XG_IN_HARDWARE {
            return Some(1);
        }

        if *dlsid == DLSID_SUPPORTS_DLS1 {
            return Some(1);
        }

        if *dlsid == DLSID_SUPPORTS_DLS2 {
            return Some(1);
        }

        if *dlsid == DLSID_SAMPLE_MEMORY_SIZE {
            return Some(u32::MAX / 4 * 3); // ~1.5GiB
        }

        if *dlsid == DLSID_MANUFACTURERS_ID {
            // See https://www.amei.or.jp/report/System_ID_e.html
            // Who are we?
            return Some(0x00004000); // a random number, no manufacturer have owned it
        }

        if *dlsid == DLSID_PRODUCT_ID {
            return Some(0x0d000721); // a random number
        }

        if *dlsid == DLSID_SAMPLE_PLAYBACK_RATE {
            return Some(self.output_sample_rate);
        }

        flog!(
            FLUID_WARN,
            "Unknown CDL query {{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
            dlsid.data1,
            dlsid.data2,
            dlsid.data3,
            dlsid.data4[0],
            dlsid.data4[1],
            dlsid.data4[2],
            dlsid.data4[3],
            dlsid.data4[4],
            dlsid.data4[5],
            dlsid.data4[6],
            dlsid.data4[7]
        );
        None
    }

    /// Translation of `fluid_dls_font::execute_cdls()`: offset is at chunk
    /// data.
    fn execute_cdls(&mut self, offset: i64, size: i32) -> DlsResult<bool> {
        let mut stack = [0u32; 64];
        let mut sp: usize = 0;

        fn push(stack: &mut [u32; 64], sp: &mut usize, value: u32) -> DlsResult<()> {
            if *sp >= stack.len() {
                return Err(DlsException::runtime("CDL stack overflow"));
            }

            stack[*sp] = value;
            *sp += 1;
            Ok(())
        }

        fn pop(stack: &[u32; 64], sp: &mut usize) -> DlsResult<u32> {
            if *sp == 0 {
                return Err(DlsException::runtime("CDL stack underflow"));
            }

            *sp -= 1;
            Ok(stack[*sp])
        }

        if self.fcbs_fseek(offset, SEEK_SET) != FLUID_OK {
            return Err(DlsException::runtime("Failed to seek to CDL operations"));
        }

        let mut pc: i32 = 0;
        // (rax and rbx: these two are just temp vars)

        while pc < size {
            let opcode = self.read16()?;

            match opcode {
                // Assign
                0x0010 => {
                    // DLS_CDL_CONST
                    let rax = self.read32()?;
                    push(&mut stack, &mut sp, rax)?;
                    pc += 4;
                }

                // Unary operators
                0x000F => {
                    // DLS_CDL_NOT (logical)
                    let rax = pop(&stack, &mut sp)?;
                    push(&mut stack, &mut sp, (rax == 0) as u32)?;
                }

                // Binary operators
                0x0001..=0x000E => {
                    let rax = pop(&stack, &mut sp)?; // X, top
                    let rbx = pop(&stack, &mut sp)?; // Y, prev
                    let value = match opcode {
                        0x0001 => rax & rbx,             // DLS_CDL_AND
                        0x0002 => rax | rbx,             // DLS_CDL_OR
                        0x0003 => rax ^ rbx,             // DLS_CDL_XOR
                        0x0004 => rax.wrapping_add(rbx), // DLS_CDL_ADD
                        0x0005 => rax.wrapping_sub(rbx), // DLS_CDL_SUBTRACT
                        0x0006 => rax.wrapping_mul(rbx), // DLS_CDL_MULTIPLY
                        0x0007 => {
                            // DLS_CDL_DIVIDE
                            if rbx == 0 {
                                return Err(DlsException::runtime("CDL division by zero"));
                            }

                            rax / rbx
                        }
                        0x0008 => (rax != 0 && rbx != 0) as u32, // DLS_CDL_LOGICAL_AND
                        0x0009 => (rax != 0 || rbx != 0) as u32, // DLS_CDL_LOGICAL_OR
                        0x000A => (rax < rbx) as u32,            // DLS_CDL_LT
                        0x000B => (rax <= rbx) as u32,           // DLS_CDL_LE
                        0x000C => (rax > rbx) as u32,            // DLS_CDL_GT
                        0x000D => (rax >= rbx) as u32,           // DLS_CDL_GE
                        _ => (rax == rbx) as u32,                // DLS_CDL_EQ
                    };
                    push(&mut stack, &mut sp, value)?;
                }

                // Query
                0x0011 | 0x0012 => {
                    // DLS_CDL_QUERY, DLS_CDL_QUERY_SUPPORTED
                    let mut dlsid = Dlsid::default();
                    read_guid(self, &mut dlsid)?;
                    let result = self.eval_dlsid_query(&dlsid);

                    if opcode == 0x0011 {
                        // Query
                        match result {
                            Some(value) => push(&mut stack, &mut sp, value)?,
                            None => {
                                return Err(DlsException::runtime(format!(
                                    "CDL query for unsupported DLSID, pc=0x{:x}",
                                    pc
                                )));
                            }
                        }
                    }

                    // FIXME (upstream): a QUERY pushes the "supported" flag
                    // over its value too (DLS-2.2 has it push the value
                    // only), so an expression that uses the value sees the
                    // flag instead.
                    push(&mut stack, &mut sp, result.is_some() as u32)?; // Query_Supported
                    pc += 16; // sizeof(DLSID)
                }

                _ => {
                    // SIGILL lol
                    return Err(DlsException::runtime(format!(
                        "Unknown CDL opcode 0x{:04x}",
                        opcode
                    )));
                }
            } // switch(opcode) end

            pc += 2;
        } // while pc end

        if pc > size {
            return Err(DlsException::runtime("CDL chunk too early end of chunk"));
        }

        Ok(pop(&stack, &mut sp)? != 0)
    }

    // wave

    /// Translation of `fluid_dls_font::parse_wvpl()`: offset is at chunk
    /// header.
    fn parse_wvpl(&mut self, offset: i64) -> DlsResult<()> {
        // get basic information of wvpl chunk
        self.fseek(offset, SEEK_SET)?;
        let mut chunk = RiffChunk::default();
        let headersize = read_chunk(self, &mut chunk)?;

        if offset + headersize as i64 + chunk.size as i64 > self.filesize {
            return Err(DlsException::runtime("DLS wvpl chunk exceeds file size"));
        }

        if headersize != 12 {
            flog!(
                FLUID_WARN,
                "Nonstandard 'wvpl' chunk; 'LIST[wvpl]' is legal"
            );
        }

        // iterate ptbl
        for i in 0..self.poolcues.len() {
            let pos = self.poolcues[i];
            let mut sample = DlsSample::default();

            let result = self.parse_wave(offset + headersize as i64 + pos as i64, &mut sample);
            self.samples.push(sample);
            if let Err(exc) = result {
                return Err(DlsException::runtime(format!(
                    "Exception thrown while parsing LIST[wave] at offset 0x{:x}, ptbl index {}",
                    (offset + headersize as i64 + pos as i64) as u64,
                    i as u32
                ))
                .nest(exc));
            }
        }
        Ok(())
    }

    /// Translation of `fluid_dls_font::parse_wave()`.
    fn parse_wave(&mut self, offset: i64, sample: &mut DlsSample) -> DlsResult<()> {
        let mut contains_data = false;
        let mut fmt_tag: u16 = 0;
        let mut bits_per_sample: u16 = 0;

        self.visit_subchunks(offset, WAVE_FCC, |this, subchunk, headersize, pos| {
            match subchunk.id {
                INFO_FCC => {
                    sample.name =
                        this.read_name_from_info_entries(pos + headersize as i64, subchunk.size)?;
                }

                DLID_FCC | GUID_FCC | FACT_FCC | CUE_FCC => {}

                FMT_FCC => {
                    // See WAVEFORMAT struct (mmreg.h)
                    let temp16 = this.read16()?; // read formatTag
                    fmt_tag = temp16;

                    if fmt_tag != WAVE_FORMAT_PCM {
                        return Err(DlsException::runtime(format!(
                            "Unsupported wave format {} (without libsndfile)",
                            fmt_tag
                        )));
                    }

                    let temp16 = this.read16()?; // read channels

                    if temp16 != 1 {
                        return Err(DlsException::runtime(format!(
                            "Unsupported wave channel count {}",
                            temp16
                        )));
                    }

                    sample.samplerate = this.read32()?; // read sampleRate
                    this.read32()?; // read avgBytesPerSec
                    this.read16()?; // read blockAlign
                    let temp16 = this.read16()?; // read bitsPerSample

                    if temp16 != 8 && temp16 != 16 {
                        return Err(DlsException::runtime(format!(
                            "Unsupported wave bits per sample {}",
                            temp16
                        )));
                    }

                    bits_per_sample = temp16;
                    // probably a cbSize field for WAVEFORMATEX
                }

                WSMP_FCC => {
                    let wsmp = sample.wsmp.insert(DlsWsmp::default());
                    if this.parse_wsmp(pos + headersize as i64, wsmp)? != subchunk.size {
                        return Err(DlsException::runtime(
                            "DLS wsmp chunk in wave chunk has corrupted size",
                        ));
                    }
                }

                DATA_FCC => {
                    contains_data = true;

                    if fmt_tag == 0 || bits_per_sample == 0 {
                        return Err(DlsException::runtime(
                            "DLS fmt chunk must exist in wave chunk and be prior to data chunk",
                        ));
                    }

                    if fmt_tag != WAVE_FORMAT_PCM {
                        return Ok(()); // leave data to libsndfile
                    }

                    let bytes = (bits_per_sample / 8) as u32;
                    if subchunk.size % bytes != 0 {
                        return Err(DlsException::runtime(
                            "DLS data chunk not align to bitsPerSample",
                        ));
                    }

                    let samplelen = subchunk.size / bytes;
                    sample.start = this.sampledata.len() as u32;
                    sample.end = sample.start.wrapping_add(samplelen);
                    // FIXME (upstream): the size is not checked against the
                    // file's, so a small file can make this allocate (and
                    // zero) up to 4 GiB before the read fails.
                    let newlen = this.sampledata.len() + samplelen as usize;
                    try_resize(&mut this.sampledata, newlen)?;

                    let mut buffer = [0u8; 4096];
                    let mut remaining = subchunk.size;
                    let mut destination = sample.start as usize;

                    while remaining > 0 {
                        let c = if remaining as usize > buffer.len() {
                            buffer.len()
                        } else {
                            remaining as usize
                        };

                        if this.fread(&mut buffer[..c]) != FLUID_OK {
                            flog!(FLUID_ERR, "fcbs::fread failed when reading DLS data chunk");
                            return Err(DlsException::std());
                        }

                        let n = c / bytes as usize;
                        read_data_lpcm(
                            &mut this.sampledata[destination..destination + n],
                            &buffer,
                            c,
                            bits_per_sample,
                        )?;

                        destination += n;
                        remaining -= c as u32;
                    }
                }

                _ => {
                    flog!(
                        FLUID_WARN,
                        "{}",
                        c_str(&format!(
                            "Unknown DLS chunk in LIST[wave] '{}'",
                            fmt_4cc(subchunk.id)
                        ))
                    );
                }
            }
            Ok(())
        })?;

        if !contains_data {
            return Err(DlsException::runtime(
                "DLS data chunk must exist in wave chunk",
            ));
        }

        // (a format other than PCM failed above: the libsndfile path,
        // parse_wave_sndfile(), is left out)
        Ok(())
    }

    /// Translation of `fluid_dls_font::parse_wsmp()`: offset is at
    /// wsmpnnnn^... (chunk data); returns the size the chunk should have.
    fn parse_wsmp(&mut self, offset: i64, wsmp: &mut DlsWsmp) -> DlsResult<u32> {
        self.fseek(offset, SEEK_SET)?;

        let mut cbsize = self.read32()?;

        if cbsize < 20 {
            // This is seen in Crystal's DLS. See CRS1_FCC comment.
            flog!(
                FLUID_WARN,
                "DLS wsmp chunk cbSize < 20. The file is probably corrupted."
            );
            cbsize = 20;
        }

        wsmp.unity_note = self.read16()?;
        wsmp.fine_tune = self.read16()? as i16;
        wsmp.gain = self.read32()? as i32;
        self.fskip(4)?; // fulOptions
        let loops = self.read32()?;

        if loops > 1 {
            return Err(DlsException::runtime(
                "DLS wsmp chunk has more than one loop",
            ));
        }

        if loops == 0 {
            return Ok(cbsize);
        }

        let mut loopcbsize = self.read32()?;

        if loopcbsize < 16 {
            // This is also seen in Crystal's DLS. See CRS1_FCC comment.
            flog!(
                FLUID_WARN,
                "DLS wsmp chunk loop cbSize < 16. The file is probably corrupted."
            );
            loopcbsize = 16;
        }

        wsmp.loop_type = self.read32()?;
        wsmp.loop_start = self.read32()?;
        wsmp.loop_length = self.read32()?;
        Ok(cbsize.wrapping_add(loopcbsize))
    }

    // lins

    /// Translation of `fluid_dls_font::parse_lins()`: offset is at chunk
    /// header.
    fn parse_lins(&mut self, offset: i64) -> DlsResult<()> {
        self.visit_subchunks(offset, LINS_FCC, |this, subchunk, _headersize, pos| {
            if subchunk.id == CRS1_FCC || subchunk.id == CRS2_FCC {
                return Ok(());
            }

            if subchunk.id != INS_FCC {
                flog!(
                    FLUID_WARN,
                    "{}",
                    c_str(&format!(
                        "Unknown DLS chunk '{}' ofs=0x{:x} in LIST[lins]",
                        fmt_4cc(subchunk.id),
                        pos as u64
                    ))
                );
                return Ok(());
            }

            // (the instrument is parsed out of the vector, then put back)
            this.instruments.push(DlsInstrument::default());
            let index = this.instruments.len() - 1;
            let mut instrument = std::mem::take(&mut this.instruments[index]);
            let result = this.parse_ins(pos, &mut instrument);
            this.instruments[index] = instrument;
            result
        })?;
        Ok(())
    }

    /// The `parse_lart(pos, articulations[index])` of an articulation
    /// (parsed out of the vector, then put back).
    fn parse_lart_at(&mut self, offset: i64, index: usize) -> DlsResult<bool> {
        let mut articulation = std::mem::take(&mut self.articulations[index]);
        let result = self.parse_lart(offset, &mut articulation);
        self.articulations[index] = articulation;
        result
    }

    /// Translation of `fluid_dls_font::parse_ins()`.
    fn parse_ins(&mut self, offset: i64, instrument: &mut DlsInstrument) -> DlsResult<()> {
        let mut articulation_index = usize::MAX; // -1; in C++26 we can do std::optional<T&>

        self.visit_subchunks(offset, INS_FCC, |this, subchunk, headersize, pos| {
            match subchunk.id {
                DLID_FCC | CRS1_FCC | CRS2_FCC => {}

                INFO_FCC => {
                    instrument.name =
                        this.read_name_from_info_entries(pos + headersize as i64, subchunk.size)?;
                }

                INSH_FCC => {
                    if subchunk.size != 12 {
                        return Err(DlsException::runtime("DLS insh chunk size != 12"));
                    }

                    // FIXME (upstream): the count of regions is not checked
                    // against the list's size, so a small file can make this
                    // allocate room for up to 4 G regions.
                    let temp = this.read32()?; // cRegions
                    try_reserve(&mut instrument.regions, temp as usize)?;
                    let temp = this.read32()?; // bank
                    instrument.is_drums = (temp & 0x80000000) != 0;
                    instrument.bankmsb = ((temp >> 8) & 0x7F) as i32;
                    instrument.banklsb = (temp & 0x7F) as i32;
                    let temp = this.read32()?;
                    instrument.pcnum = (temp & 0x7F) as i32;
                }

                LART_FCC | LAR2_FCC => {
                    if articulation_index == usize::MAX {
                        articulation_index = this.articulations.len();
                        this.articulations.push(DlsArticulation::default());

                        if !this.parse_lart_at(pos, articulation_index)? {
                            // bypassed by cdl
                            flog!(FLUID_DBG, "A instrument lart chunk is bypassed by cdl");
                            this.articulations.pop();
                            articulation_index = usize::MAX;
                        }

                        return Ok(());
                    }

                    this.parse_lart_at(pos, articulation_index)?;
                    // FIXME (upstream): the instrument's Key Number generator
                    // is only taken from a second articulation list: with
                    // the usual one, it is ignored.
                    instrument.keynum_scale = this.articulations[articulation_index].keynum_scale;
                }

                LRGN_FCC => {
                    this.parse_lrgn(pos, instrument)?;
                }

                _ => {
                    flog!(
                        FLUID_WARN,
                        "{}",
                        c_str(&format!(
                            "Unknown DLS chunk '{}' ofs=0x{:x} in LIST[ins]",
                            fmt_4cc(subchunk.id),
                            pos as u64
                        ))
                    );
                }
            }
            Ok(())
        })?;

        for region in &mut instrument.regions {
            if region.artindex == u32::MAX {
                region.artindex = articulation_index as u32;
            }
        }
        Ok(())
    }

    /// Translation of `fluid_dls_font::parse_lart()`: false if a
    /// conditional chunk bypasses it.
    fn parse_lart(&mut self, offset: i64, articulation: &mut DlsArticulation) -> DlsResult<bool> {
        let mut bypassed = false;

        let result = self.visit_subchunks(offset, 0, |this, subchunk, headersize, pos| {
            if subchunk.id == CDL_FCC {
                // (the chunk size is passed as an int, see the top-level
                // cdl)
                if !this.execute_cdls(pos + headersize as i64, subchunk.size as i32)? {
                    // Note (upstream): upstream sets bypassed to false here
                    // (parse_rgn() sets it to true), so an articulation list
                    // whose condition is false fails the whole file, with
                    // "std::exception" at the bottom of the chain, instead
                    // of being bypassed. Fixed here: it is bypassed.
                    bypassed = true;
                    return Err(DlsException::std());
                }

                return Ok(());
            }

            if subchunk.id != ART1_FCC && subchunk.id != ART2_FCC {
                flog!(
                    FLUID_WARN,
                    "{}",
                    c_str(&format!(
                        "Unknown DLS chunk '{}' ofs=0x{:x} in LIST[lart]",
                        fmt_4cc(subchunk.id),
                        pos as u64
                    ))
                );
                return Ok(());
            }

            this.parse_art(pos, articulation)
        });

        match result {
            Ok(_) => Ok(true),
            Err(_) if bypassed => Ok(false),
            Err(exc) => Err(exc),
        }
    }

    /// Translation of `fluid_dls_font::parse_art()`.
    fn parse_art(&mut self, offset: i64, articulation: &mut DlsArticulation) -> DlsResult<()> {
        self.fseek(offset, SEEK_SET)?;
        let mut chunk = RiffChunk::default();
        let headersize = read_chunk(self, &mut chunk)?;

        if chunk.id != ART1_FCC && chunk.id != ART2_FCC {
            // this should never happen!
            return Err(DlsException::runtime("Expected 'art1' or 'art2' chunk"));
        }

        if headersize != 8 {
            flog!(
                FLUID_WARN,
                "Nonstandard LIST[art1/2] chunk; 'art1/2' is legal ofs=0x{:x}",
                offset as u64
            );
        }

        let is_art1 = chunk.id == ART1_FCC;

        let cbsize = self.read32()?;

        if cbsize < 8 {
            return Err(DlsException::runtime("art chunk cbSize < 8"));
        }

        let connblocks = self.read32()?;

        if connblocks > (u32::MAX - cbsize) / 12 {
            return Err(DlsException::runtime(
                "Too many ConnectionBlocks are contained in the articulator chunk.",
            ));
        }

        if cbsize + connblocks * 12 != chunk.size {
            return Err(DlsException::runtime("art chunk has corrupted size"));
        }

        self.fskip((cbsize - 8) as i64)?;

        for _ in 0..connblocks {
            let source = self.read16()?;
            let control = self.read16()?;
            let dest = self.read16()?;
            let mut trans = self.read16()?;

            if is_art1 {
                // DLS-1 uses concave transform for CC 7 and 11
                // in DLS-2 the transform is applied to source

                // DLS-2 transform: 0b00nnnn'00nnnn'0000
                trans = (((trans as u32) << 10) | ((trans as u32) << 4)) as u16;
                let mut dlsv2trans = DlsTransform::new(trans);

                if source == CONN_SRC_LFO || source == CONN_SRC_VIBRATO {
                    dlsv2trans.src_bip = true;
                }

                if control == CONN_SRC_LFO || control == CONN_SRC_VIBRATO {
                    dlsv2trans.ctl_bip = true;
                }

                trans = dlsv2trans.to_u16();
            }

            let scale = self.read32()? as i32;
            convert_dls_connectionblock_to_art(articulation, source, control, dest, trans, scale);
        }
        Ok(())
    }

    /// Translation of `fluid_dls_font::parse_lrgn()`.
    fn parse_lrgn(&mut self, offset: i64, instrument: &mut DlsInstrument) -> DlsResult<()> {
        self.visit_subchunks(offset, LRGN_FCC, |this, subchunk, _headersize, pos| {
            match subchunk.id {
                RGN_FCC | RGN2_FCC => {
                    let mut region = DlsRegion::default();

                    if this.parse_rgn(pos, &mut region)? {
                        instrument.regions.push(region);
                    } else {
                        // bypassed by cdl
                        flog!(
                            FLUID_DBG,
                            "Region ofs=0x{:x} is bypassed by cdl",
                            pos as u64
                        );
                    }
                }

                CRS1_FCC | CRS2_FCC => {
                    // see CRS1 and CRS2_FCC's comment
                }

                _ => {
                    flog!(
                        FLUID_WARN,
                        "{}",
                        c_str(&format!(
                            "Unknown DLS chunk '{}' ofs=0x{:x} in LIST[lrgn]",
                            fmt_4cc(subchunk.id),
                            pos as u64
                        ))
                    );
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Translation of `fluid_dls_font::parse_rgn()`: false if a
    /// conditional chunk bypasses the region.
    fn parse_rgn(&mut self, offset: i64, region: &mut DlsRegion) -> DlsResult<bool> {
        // (static int self_exclusive_class = 65536; // exclusive number for
        // self-exclusive regions: unused, see rgnh)
        let mut articulation_index = usize::MAX; // -1
        let mut range = FluidZoneRange::default();

        let mut bypassed = false;

        let result = self.visit_subchunks(offset, 0, |this, subchunk, headersize, pos| {
            match subchunk.id {
                INFO_FCC | CRS1_FCC | CRS2_FCC => {}

                WLNK_FCC => {
                    this.fskip(8)?; // fluidsynth does not implement phase-locking and multichannel output
                    region.sampleindex = this.read32()?;

                    if region.sampleindex as usize >= this.samples.len() {
                        return Err(DlsException::runtime(format!(
                            "Sample index {} is out of range",
                            region.sampleindex
                        )));
                    }
                }

                CDL_FCC => {
                    // (the chunk size is passed as an int, see the top-level
                    // cdl)
                    if !this.execute_cdls(pos + headersize as i64, subchunk.size as i32)? {
                        bypassed = true;
                        return Err(DlsException::std());
                    }
                }

                LART_FCC | LAR2_FCC => {
                    if articulation_index == usize::MAX {
                        articulation_index = this.articulations.len();
                        this.articulations.push(DlsArticulation::default());

                        if !this.parse_lart_at(pos, articulation_index)? {
                            flog!(FLUID_DBG, "A region lart chunk is bypassed by cdl");
                            this.articulations.pop();
                            articulation_index = usize::MAX;
                        }

                        return Ok(());
                    }

                    this.parse_lart_at(pos, articulation_index)?;

                    if this.articulations[articulation_index].keynum_scale != 1.0f32 as FluidReal {
                        flog!(
                            FLUID_WARN,
                            "Key Number Generator is not allowed in region articulation, ignoring"
                        );
                    }
                }

                RGNH_FCC => {
                    range.keylo = this.read16()? as i32; // key low
                    range.keyhi = this.read16()? as i32; // key high
                    range.vello = this.read16()? as i32; // vel low
                    range.velhi = this.read16()? as i32; // vel high
                    let _options = this.read16()?; // fusOptions

                    // ((fusOptions & F_RGN_OPTION_SELFNONEXCLUSIVE) == 0 is a
                    // self-exclusive region:
                    // implement this flag doesn't make sense
                    // region.exclusive_class = self_exclusive_class++;)

                    let temp = this.read16()?; // keyGroup

                    if temp != 0 {
                        region.exclusive_class = temp as i32;
                    }

                    // usLayer is useless
                }

                // DLS-2 1.14.6 Each region contains at minimum a <rgnh-ck> region header chunk and a <wlnk-ck> wave link chunk.
                // It may also **optionally** contain a <wsmp-ck> wave sample chunk. ...
                // DLS-2 2.2 <rgn-list> -> ... <wsmp-ck> ...
                // DLS-2 2.2 "the structure tree" ... rgn -> ... wsmp (optional) ...
                // DLS-2 2.8 Other chunks at the same nesting level include a <wsmp-ck> wave sample chunk.
                WSMP_FCC => {
                    let wsmp = region.wsmp.insert(DlsWsmp::default());
                    this.parse_wsmp(pos + headersize as i64, wsmp)?;
                }

                _ => {
                    flog!(
                        FLUID_WARN,
                        "{}",
                        c_str(&format!(
                            "Unknown DLS chunk '{}' ofs=0x{:x} in LIST[rgn]",
                            fmt_4cc(subchunk.id),
                            pos as u64
                        ))
                    );
                }
            }
            Ok(())
        });

        match result {
            Ok(_) => {}
            Err(_) if bypassed => return Ok(false),
            Err(exc) => return Err(exc),
        }

        region.range = Arc::new(range);
        region.artindex = articulation_index as u32;

        Ok(true)
    }

    // pgal

    /// Translation of `fluid_dls_font::parse_pgal()`: offset is at chunk
    /// data.
    fn parse_pgal(&mut self, offset: i64, size: i32) -> DlsResult<()> {
        if size < 4 + 128 + 4 {
            return Err(DlsException::runtime(format!(
                "DLS pgal chunk is too small: {} bytes",
                size
            )));
        }

        self.fseek(offset, SEEK_SET)?;

        // drum note aliasing table
        let version = self.read32()?;

        if version == 0x03020100 {
            // there is actually no version field, this is the drum note aliasing table
            self.fseek(-4, SEEK_CUR)?;
        }

        let mut drum_note_aliasing = [0u8; 128];
        let read = self.fread(&mut drum_note_aliasing);
        self.drum_note_aliasing = Some(drum_note_aliasing);

        if read != FLUID_OK {
            return Err(DlsException::runtime(
                "Failed to read drum note aliasing table",
            ));
        }

        // melodic instrument aliasing table
        let n_alias = self.read32()?;

        for i in 0..n_alias {
            let bank2 = self.read16()?;
            let pc2 = self.read8()?;
            self.fskip(1)?;

            let bank = self.read16()?;
            let pc = self.read8()?;
            self.fskip(1)?;

            let found = self.instruments.iter_mut().find(|instrument| {
                instrument.banklsb == (bank & 0x7F) as i32
                    && instrument.bankmsb == ((bank >> 7) & 0x7F) as i32
                    && instrument.pcnum == pc as i32
                    && !instrument.is_drums
            });

            if let Some(instrument) = found {
                instrument.aliases.push(DlsInstrumentAlias {
                    pcnum: pc2 as i32,
                    bankmsb: ((bank2 >> 7) & 0x7F) as i32,
                    banklsb: (bank2 & 0x7F) as i32,
                });
                self.total_presets += 1;
                continue;
            }

            flog!(
                FLUID_WARN,
                "DLS pgal aliasing from non-existing instrument at entry {}: \
                 bank {}/{} pc {} -> bank {}/{} pc {}",
                i,
                (bank >> 7) & 0x7F,
                bank & 0x7F,
                pc,
                (bank2 >> 7) & 0x7F,
                bank2 & 0x7F,
                pc2
            );
        }
        Ok(())
    }
}

#[cfg(test)]
impl DlsFont {
    /// The lines of the reference program's dump of a loaded font (a
    /// function the program adds to upstream's fluid_dls.cpp): the
    /// instruments in their order, the articulations, the samples and the
    /// drum note aliasing table, each number as C's printf() prints it and
    /// each `fluid_real_t` as its bits.
    pub(crate) fn dump(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut instruments: Vec<&Arc<DlsInstrument>> = Vec::new();

        // (each instrument's preset, then its aliases')
        for preset in &self.presets {
            if !instruments
                .last()
                .is_some_and(|last| Arc::ptr_eq(last, &preset.instrument))
            {
                instruments.push(&preset.instrument);
            }
        }

        for (i, ins) in instruments.iter().enumerate() {
            out.push(format!(
                "  instrument {} '{}' msb={} lsb={} pc={} drums={} keynum={:016x} regions={}",
                i,
                c_str(&ins.name),
                ins.bankmsb,
                ins.banklsb,
                ins.pcnum,
                ins.is_drums as i32,
                ins.keynum_scale.to_bits(),
                ins.regions.len()
            ));

            for r in &ins.regions {
                out.push(format!(
                    "   region keys={}-{} vels={}-{} excl={} art={} sample={} mode={} gain={:016x}",
                    r.range.keylo,
                    r.range.keyhi,
                    r.range.vello,
                    r.range.velhi,
                    r.exclusive_class,
                    r.artindex,
                    r.sampleindex,
                    r.samplemode_inherited,
                    r.gain_inherited.to_bits()
                ));

                if let Some(w) = &r.wsmp {
                    out.push(format!(
                        "    wsmp unity={} fine={} gain={} type={} start={} length={}",
                        w.unity_note, w.fine_tune, w.gain, w.loop_type, w.loop_start, w.loop_length
                    ));
                }
            }
        }

        // (the instruments share the font's articulations)
        let articulations = instruments
            .first()
            .map(|ins| &ins.articulations[..])
            .unwrap_or(&[]);

        for (i, a) in articulations.iter().enumerate() {
            out.push(format!(
                "  articulation {} keynum={:016x} k2p={:016x} mods={}",
                i,
                a.keynum_scale.to_bits(),
                a.key_to_pitch.amount.to_bits(),
                a.mods.len()
            ));

            for (g, value) in a.gens.iter().enumerate() {
                if let Some(value) = value {
                    out.push(format!("   gen {} {:016x}", g, value.to_bits()));
                }
            }

            for m in &a.mods {
                out.push(format!(
                    "   mod dest={} src1={} flags1={} src2={} flags2={} trans={} amount={:016x}",
                    m.dest,
                    m.src1,
                    m.flags1,
                    m.src2,
                    m.flags2,
                    m.trans,
                    m.amount.to_bits()
                ));
            }
        }

        for (i, s) in self.samples_fluid.iter().enumerate() {
            out.push(format!(
                "  sample {} '{}' start={} end={} loop={}-{} rate={} pitch={} adj={} type={} mods={}",
                i,
                s.name,
                s.start,
                s.end,
                s.loopstart,
                s.loopend,
                s.samplerate,
                s.origpitch,
                s.pitchadj,
                s.sampletype,
                s.default_modulators.is_some() as i32
            ));
        }

        if let Some(table) = &self.drum_note_aliasing {
            for (k, row) in table.chunks(16).enumerate() {
                let mut line = format!("  aliasing {}:", k * 16);
                for note in row {
                    line += &format!(" {}", note);
                }
                out.push(line);
            }
        }

        out
    }
}

// fluid sfloader interfaces implementation

// (`fluid_dls_loader_data`, `new_fluid_dls_loader()` and
// `fluid_dls_loader_delete()` are `SfLoader`'s: see sfont.rs.)

/// Translation of `fluid_dls_default_mod_list()`: the default modulators
/// of a DLS file, in the order of upstream's list (each one is prepended).
fn fluid_dls_default_mod_list() -> Vec<FluidMod> {
    let def_mods = [
        // skip default_vel2filter_mod
        // skip default_at2viblfo_mod
        &DEFAULT_VEL2ATT_MOD,
        &DEFAULT_MOD2VIBLFO_MOD,
        &DEFAULT_ATT_MOD,
        &DEFAULT_PAN_MOD,
        &DEFAULT_EXPR_MOD,
        &DLS_DEFAULT_REVERB_MOD,
        &DLS_DEFAULT_CHORUS_MOD,
        &DLS_DEFAULT_PITCH_BEND_MOD,
        &CUSTOM_BALANCE_MOD,
    ];

    let mut list = Vec::with_capacity(def_mods.len());

    // (Note (upstream): new_fluid_mod() failing throws std::bad_alloc out
    // of the noexcept fluid_dls_loader_load(), which terminates; Rust
    // aborts on that allocation failure too.)
    for def_mod in def_mods {
        list.insert(0, *def_mod);
    }

    list
}

/// Translation of `fluid_dls_loader_load()`.
pub(crate) fn fluid_dls_loader_load(loader: &SfLoader, filename: &str) -> Option<FluidSfont> {
    // (new_fluid_sfont(): the SoundFont is made with its font below)
    let default_mod_list = Some(Arc::new(fluid_dls_default_mod_list()));

    let mut sample_rate: u32 = 44100;
    let mut try_mlock = false;
    let settings = &loader.settings;

    let mut rate = 0.0;

    if settings.getnum("synth.sample-rate", &mut rate) == FLUID_OK {
        sample_rate = rate as u32;
    }

    let mut mlock = 0;

    if settings.getint("synth.lock-memory", &mut mlock) == FLUID_OK {
        try_mlock = mlock != 0;
    }

    let dlsfont = new_fluid_dls_font(
        &loader.file_callbacks,
        filename,
        sample_rate,
        try_mlock,
        default_mod_list.clone(),
    )?;

    // (fluid_sfont_set_data())
    Some(FluidSfont {
        data: SfontData::Dls(Box::new(dlsfont)),
        id: 0,
        bankofs: 0,
        default_mod_list,
    })
}

// fluid_sfont_t interface

impl DlsFont {
    /// Translation of `fluid_dls_sfont_get_name()`.
    pub(crate) fn get_name(&self) -> &str {
        &self.filename
    }

    /// Translation of `fluid_dls_sfont_get_preset()`: `bank_select` is the
    /// synth's.
    pub(crate) fn get_preset(
        &self,
        bank: i32,
        prenum: i32,
        bank_select: i32,
    ) -> Option<Arc<DlsPreset>> {
        for inst in &self.presets {
            if inst.get_banknum(bank_select) == bank && inst.pcnum == prenum {
                return Some(inst.clone());
            }
        }

        if bank_select == FLUID_BANK_STYLE_MMA {
            for inst in &self.presets {
                // Drum bank fallback for MMA bank style
                if bank == DRUM_INST_BANK && inst.is_drums && inst.pcnum == prenum {
                    return Some(inst.clone());
                }
            }
        }

        if bank_select == FLUID_BANK_STYLE_GM {
            for inst in &self.presets {
                // Melodic bank fallback for GM2 bank style
                if bank == 0 && !inst.is_drums && inst.bankmsb == 0x79 && inst.pcnum == prenum {
                    return Some(inst.clone());
                }
            }
        }

        None
    }

    /// Translation of `fluid_dls_iteration_start()`.
    pub(crate) fn iteration_start(&mut self) {
        self.fluid_preset_iterator = 0;
    }

    /// Translation of `fluid_dls_iteration_next()`.
    pub(crate) fn iteration_next(&mut self) -> Option<Arc<DlsPreset>> {
        let preset = self.presets.get(self.fluid_preset_iterator)?.clone();
        self.fluid_preset_iterator += 1;
        Some(preset)
    }
}

// (fluid_dls_sfont_delete(), which fails while a sample is in use, is
// `DlsFont`'s `Drop`: the voices hold the samples they use, and SoundFont
// unloading is left out.)

impl DlsPreset {
    /// Translation of `fluid_dls_preset_get_name()`.
    pub(crate) fn get_name(&self) -> &str {
        c_str(&self.instrument.name)
    }

    /// Translation of `fluid_dls_preset_get_banknum()`: `bank_select` is
    /// the synth's.
    pub(crate) fn get_banknum(&self, bank_select: i32) -> i32 {
        // see fluid_synth.h enum fluid_midi_bank_select
        // see fluid_chan.c fluid_channel_set_bank_msb()
        if bank_select == FLUID_BANK_STYLE_GM {
            if self.is_drums || self.bankmsb == 0x78 {
                // GM2 rhythm bank
                return DRUM_INST_BANK;
            }

            return self.bankmsb; // to always match bank 0 for GM
        }

        if bank_select == FLUID_BANK_STYLE_GS {
            return self.bankmsb + if self.is_drums { DRUM_INST_BANK } else { 0 };
        }

        if bank_select == FLUID_BANK_STYLE_XG {
            if self.is_drums {
                // see https://github.com/FluidSynth/fluidsynth/issues/1524
                // see fluid_chan.c fluid_channel_set_bank_msb()
                return 128;
            }

            return self.banklsb;
        }

        // if (synth->bank_select == FLUID_BANK_STYLE_MMA)
        if self.is_drums {
            return DRUM_INST_BANK * 128;
        }

        self.banklsb + (128 * self.bankmsb)
    }

    /// Translation of `fluid_dls_preset_get_num()`.
    pub(crate) fn get_num(&self) -> i32 {
        self.pcnum
    }
}

impl FluidSynthInner {
    /// Translation of `fluid_dls_preset_noteon()`.
    pub(crate) fn fluid_dls_preset_noteon(
        &mut self,
        preset: &DlsPreset,
        chan: i32,
        key: i32,
        vel: i32,
    ) -> i32 {
        let dlspreset = &*preset.instrument;
        let channel_type = self.channel[chan as usize].channel_type;

        // Things affect region selection process:
        // 1. Subtonal tuning, see https://github.com/FluidSynth/fluidsynth/issues/926
        // 2. Key Number Generator
        let mut tuned_key_f = key as FluidReal;

        // this is copied from fluid_defsfont
        if channel_type == CHANNEL_TYPE_MELODIC {
            tuned_key_f = self.channel[chan as usize].get_key_pitch(key) / 100.0f32 as FluidReal;
        }

        tuned_key_f *= dlspreset.keynum_scale;

        // Note (upstream): static_cast<int>(std::round()) of a key scaled
        // by an absurd Key Number generator is undefined; here it is
        // INT_MIN, as on x86.

        // key with subtonal tuning and key number generator applied
        let mut tuned_key = c_double_to_int(tuned_key_f.round());
        // key with only key number generator applied
        let adjusted_key;
        if channel_type == CHANNEL_TYPE_DRUM {
            if let Some(drum_note_aliasing) = &dlspreset.drum_note_aliasing {
                tuned_key = drum_note_aliasing[tuned_key.clamp(0, 127) as usize] as i32;
            }
            // drum channel has no subtonal tuning, so adjusted_key == tuned_Key
            adjusted_key = tuned_key;
        } else {
            adjusted_key = c_double_to_int((key as FluidReal * dlspreset.keynum_scale).round());
        }

        for region in &dlspreset.regions {
            let sample = &dlspreset.samples_fluid[region.sampleindex as usize];
            // Check for zero-length samples, typically caused by samples that failed fluid_sample_validate()
            if !fluid_zone_inside_range(&region.range, tuned_key, vel) || sample.start == sample.end
            {
                continue;
            }

            let Some(v) = self.fluid_synth_alloc_voice_local(
                sample,
                chan,
                adjusted_key,
                vel,
                Some(region.range.clone()),
            ) else {
                return FLUID_FAILED;
            };

            let voice = &mut self.voice[v];

            if region.artindex != u32::MAX {
                let art = &dlspreset.articulations[region.artindex as usize];

                for (i, gen) in art.gens.iter().enumerate() {
                    if let Some(value) = gen {
                        fluid_voice_gen_set(voice, &mut self.eventhandler, i, *value as f32);
                    }
                }

                // this should be the count of default mods to be probably overwritten
                let existing_mod_count = voice.mod_count;

                for mod_ in &art.mods {
                    fluid_voice_add_mod_local(
                        voice,
                        mod_,
                        FLUID_VOICE_OVERWRITE,
                        existing_mod_count,
                    );
                }

                // See also https://github.com/FluidSynth/fluidsynth/pull/1626 conversation for "Key Number to Pitch" articulation implementation
                fluid_voice_gen_set(
                    voice,
                    &mut self.eventhandler,
                    GEN_SCALETUNE,
                    (art.key_to_pitch.amount / 128.0) as f32,
                );
            }

            if let Some(wsmp) = &region.wsmp {
                fluid_voice_gen_set(
                    voice,
                    &mut self.eventhandler,
                    GEN_OVERRIDEROOTKEY,
                    wsmp.unity_note as f32,
                );
                fluid_voice_gen_incr(
                    voice,
                    GEN_FINETUNE,
                    (wsmp.fine_tune as i32 - sample.pitchadj) as f32,
                );
                // Note (upstream): -wsmp.gain overflows for INT_MIN (here it
                // wraps, as on x86); and it is divided as a float, where the
                // gain a region inherits is divided as a fluid_real_t.
                fluid_voice_gen_incr(
                    voice,
                    GEN_ATTENUATION,
                    wsmp.gain.wrapping_neg() as f32 / 65536.0f32,
                );

                if wsmp.loop_length != 0 {
                    if wsmp.loop_type == 0 {
                        fluid_voice_gen_set(voice, &mut self.eventhandler, GEN_SAMPLEMODE, 1.0);
                    } else if wsmp.loop_type == 1 {
                        fluid_voice_gen_set(voice, &mut self.eventhandler, GEN_SAMPLEMODE, 3.0);
                    } else {
                        flog!(
                            FLUID_WARN,
                            "invalid loop type of region wsmp, set to loop and release"
                        );
                        fluid_voice_gen_set(voice, &mut self.eventhandler, GEN_SAMPLEMODE, 3.0);
                    }
                } else {
                    fluid_voice_gen_set(voice, &mut self.eventhandler, GEN_SAMPLEMODE, 0.0);
                }

                fluid_voice_gen_set(
                    voice,
                    &mut self.eventhandler,
                    GEN_STARTLOOPADDROFS,
                    (sample.start.wrapping_add(wsmp.loop_start) as i32)
                        .wrapping_sub(sample.loopstart as i32) as f32,
                );
                fluid_voice_gen_set(
                    voice,
                    &mut self.eventhandler,
                    GEN_ENDLOOPADDROFS,
                    (sample
                        .start
                        .wrapping_add(wsmp.loop_start)
                        .wrapping_add(wsmp.loop_length) as i32)
                        .wrapping_sub(sample.loopend as i32) as f32,
                );
            } else {
                fluid_voice_gen_incr(voice, GEN_ATTENUATION, (-region.gain_inherited) as f32);
                fluid_voice_gen_set(
                    voice,
                    &mut self.eventhandler,
                    GEN_SAMPLEMODE,
                    region.samplemode_inherited as f32,
                );
            }

            fluid_voice_gen_set(
                voice,
                &mut self.eventhandler,
                GEN_EXCLUSIVECLASS,
                region.exclusive_class as f32,
            );

            // FIXME (upstream): the generator defaults an AWE32 NRPN
            // overrides (fluid_channel_get_override_gen_default()) are not
            // applied to DLS voices, as fluid_defpreset_noteon() applies
            // them; FluidSynth 2.6.1 fixes it (a953e62).

            self.fluid_synth_start_voice(v);
        }

        FLUID_OK
    }
}

// (fluid_dls_preset_free() does nothing: the presets are the font's.)
