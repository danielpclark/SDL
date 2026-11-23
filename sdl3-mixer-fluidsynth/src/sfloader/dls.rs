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
// Modified 2026-11-23: more of fluid_dls.cpp translated, in its order: the
// exceptions (`DlsException`, carried in `Result`) and log_exception(), the
// RIFF chunk reading, the DLSIDs and the conditional chunks (CDL), the
// wave pool, and the loader with its default modulators, as far as the
// constructor's parsing of the instruments (`LIST[lins]`), which is not
// translated yet: a file that gets there fails to load. The libsndfile
// path is left out, as SDL_mixer's CMake builds FluidSynth without
// libsndfile; so is pinning the sample data to RAM (mlock).

use std::collections::TryReserveError;
use std::sync::Arc;

use crate::fluidsynth_h::*;
use crate::sfloader::defsfont::FluidZoneRange;
use crate::sfloader::sfont::{FluidFileCallbacks, FluidSfont, SfFile, SfLoader};
use crate::synth::modulator::FluidMod;
use crate::synth::synth::{
    CUSTOM_BALANCE_MOD, DEFAULT_ATT_MOD, DEFAULT_EXPR_MOD, DEFAULT_MOD2VIBLFO_MOD, DEFAULT_PAN_MOD,
    DEFAULT_VEL2ATT_MOD, DLS_DEFAULT_CHORUS_MOD, DLS_DEFAULT_PITCH_BEND_MOD,
    DLS_DEFAULT_REVERB_MOD,
};
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

/// Translation of `fluid_dls_wsmp`: wsmp is used in LIST[wave] chunk and
/// LIST[rgn ] chunk, converting to different fluid structures.
#[derive(Clone, Copy, Debug, Default)]
#[allow(dead_code)] // (read once the instruments are parsed)
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
#[allow(dead_code)] // (read once the instruments are parsed)
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
#[allow(dead_code)] // (filled from the instruments)
struct DlsRegion {
    range: FluidZoneRange,
    wsmp: Option<DlsWsmp>,
    exclusive_class: i32,

    artindex: u32,
    sampleindex: u32,
    samplemode_inherited: u8, // loop_type from sample's wsmp, converted to SF2 sample mode
    gain_inherited: FluidReal, // gain from sample's wsmp, in cB
}

/// Translation of `fluid_dls_instrument_alias`.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // (filled from the instruments)
struct DlsInstrumentAlias {
    pcnum: i32,
    bankmsb: i32,
    banklsb: i32,
}

/// Translation of `fluid_dls_instrument`: SF2's instrument level does not
/// exist in DLS, preset is called instrument in DLS. (Upstream's pointers
/// to the font's samples, articulations, synth and drum note aliasing are
/// left to the presets.)
#[derive(Debug)]
#[allow(dead_code)] // (filled from the instruments)
struct DlsInstrument {
    pcnum: i32,
    bankmsb: i32,
    banklsb: i32,
    is_drums: bool,

    name: String,
    regions: Vec<DlsRegion>,

    // see fluid_dls_articulation::keynum_scale
    keynum_scale: FluidReal,

    // for 'pgal' chunk in MobileBAE DLS banks
    aliases: Vec<DlsInstrumentAlias>,
}

/// Translation of `fluid_dls_font`, as far as the instruments. (Its file
/// callbacks are the methods of its open `file`.)
struct DlsFont {
    output_sample_rate: u32, // only used for cdl eval
    #[allow(dead_code)] // (the SoundFont's name, once one loads)
    filename: String,
    /* (the SoundFont's default modulators, which the samples get) */
    #[allow(dead_code)] // (read once the instruments are parsed)
    default_mod_list: Option<Arc<Vec<FluidMod>>>,

    /* (closed when the font drops, as upstream's `on_file_exit`) */
    file: Option<Box<dyn SfFile>>,
    filesize: i64,

    // offset to chunk header
    linsoffset: i64,
    wvploffset: i64,

    // offset to chunk data
    #[allow(dead_code)] // (read once the instruments are parsed)
    pgaloffset: i64,
    #[allow(dead_code)] // (read once the instruments are parsed)
    pgalsize: i64,

    sampledata: Vec<i16>,
    poolcues: Vec<u32>, // data of ptbl

    samples: Vec<DlsSample>,
    instruments: Vec<DlsInstrument>,
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
const WSMP_FCC: u32 = fourcc(b'w', b's', b'm', b'p');
const FMT_FCC: u32 = fourcc(b'f', b'm', b't', b' ');
const DATA_FCC: u32 = fourcc(b'd', b'a', b't', b'a');

// LIST chunks
const INFO_FCC: u32 = fourcc(b'I', b'N', b'F', b'O');
const LINS_FCC: u32 = fourcc(b'l', b'i', b'n', b's');
const WVPL_FCC: u32 = fourcc(b'w', b'v', b'p', b'l');
const WAVE_FCC: u32 = fourcc(b'w', b'a', b'v', b'e');

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
            instruments: Vec::new(),
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

    /// `fluid_dls_font::parse_lins()`: offset is at chunk header. Not
    /// translated yet: a file with instruments fails to load here, with a
    /// message of the translation's own.
    fn parse_lins(&mut self, _offset: i64) -> DlsResult<()> {
        Err(DlsException::runtime(
            "DLS instruments are not supported by this translation yet",
        ))
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
        default_mod_list,
    )?;

    // (fluid_sfont_set_data(): until the instruments are translated, the
    // constructor fails before a font gets here)
    drop(dlsfont);
    None
}
