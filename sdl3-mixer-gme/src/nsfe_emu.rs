// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2005-2006 Shay Green. This module is free software; you
can redistribute it and/or modify it under the terms of the GNU Lesser
General Public License as published by the Free Software Foundation; either
version 2.1 of the License, or (at your option) any later version. This
module is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
details. You should have received a copy of the GNU Lesser General Public
License along with this module; if not, write to the Free Software Foundation,
Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA */
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! Nintendo NES/Famicom NSFE music file emulator (`Nsfe_Emu.h`,
//! `Nsfe_Emu.cpp`).
//!
//! `Nsfe_Info::load()` loads the NSF data it finds into the emulator
//! loading the NSFE file through [`NsfeInfoHost`] (upstream passes the
//! emulator, or NULL for the info-only reader).

use crate::blargg_common::{blargg_4char, get_le32, try_resize, BlarggErr};
use crate::blip_buffer::{BlipEq, BlipTime, BufId};
use crate::classic_emu::{ClassicEmu, ClassicEmuState};
use crate::data_reader::{DataReader, RemainingReader, SubsetReader, EOF_ERROR};
use crate::gme_file::{
    self, copy_field_, gme_copy_field, GmeTypeT, TrackInfo, GME_WRONG_FILE_TYPE,
};
use crate::music_emu::{
    gme_info_overrides, music_emu_unload, Equalizer, MusicEmu, MusicEmuState, Sample,
};
use crate::nsf_emu::{header, NsfEmu, HEADER_SIZE};

/// `Nsfe_Info::info_t`: an NSF header followed by the NSFE strings.
#[derive(Clone, Debug)]
pub struct NsfeInfoT {
    pub header: [u8; HEADER_SIZE],
    pub game: [u8; 256],
    pub author: [u8; 256],
    pub copyright: [u8; 256],
    pub dumper: [u8; 256],
}

impl Default for NsfeInfoT {
    fn default() -> Self {
        NsfeInfoT {
            header: [0; HEADER_SIZE],
            game: [0; 256],
            author: [0; 256],
            copyright: [0; 256],
            dumper: [0; 256],
        }
    }
}

/// Allows reading info from NSFE file without creating emulator
#[derive(Clone, Debug, Default)]
pub struct NsfeInfo {
    pub info: NsfeInfoT,
    track_name_data: Vec<u8>,
    /// (offsets into track_name_data)
    track_names: Vec<usize>,
    playlist: Vec<u8>,
    track_times: Vec<[u8; 4]>,
    /// FIXME (upstream): this is left uninitialized when there is no INFO
    /// block; it is zero here, as in upstream's patched build for the tests.
    actual_track_count_: i32,
    playlist_disabled: bool,
}

/// What `Nsfe_Info::load()` loads the NSF data into.
pub trait NsfeInfoHost {
    fn nsfe_info(&mut self) -> &mut NsfeInfo;
    /// Load the NSF data of the DATA block, `size` bytes of `input`, into
    /// the emulator, or `None` where there is no emulator.
    fn load_nsf(&mut self, input: &mut dyn DataReader, size: i64) -> Option<BlarggErr>;
    /// Whether there is an emulator to load the NSF data into.
    fn load_nsf_wanted(&self) -> bool;
}

/// The error reading `size` bytes gives when there aren't that many left:
/// checked before allocating a buffer for them, which upstream allocates
/// first (when the size from the file is huge, the allocation succeeds and
/// the read fails, as it does here without the allocation).
fn check_remain(input: &dyn DataReader, size: i64) -> BlarggErr {
    if size > 0 && size > input.remain() {
        return Err(EOF_ERROR);
    }
    Ok(())
}

/// Read multiple strings and separate into individual strings
fn read_strs(
    input: &mut dyn DataReader,
    size: i64,
    chars: &mut Vec<u8>,
    strs: &mut Vec<usize>,
) -> BlarggErr {
    check_remain(input, size)?;
    try_resize(chars, (size + 1) as usize)?;
    chars[size as usize] = 0; // in case last string doesn't have terminator
    input.read(&mut chars[..size as usize])?;

    try_resize(strs, 128)?;
    let mut count = 0usize;
    let mut i = 0i64;
    while i < size {
        if strs.len() <= count {
            try_resize(strs, count * 2)?;
        }
        strs[count] = i as usize;
        count += 1;
        while i < size && chars[i as usize] != 0 {
            i += 1;
        }
        i += 1;
    }

    try_resize(strs, count)
}

/// Copy in to out, where out has out_max characters allocated. Truncate to
/// out_max - 1 characters.
fn copy_str(input: &[u8], out: &mut [u8]) {
    let out_max = out.len();
    out[out_max - 1] = 0;
    // strncpy( out, in, out_max - 1 );
    let mut n = 0;
    while n < out_max - 1 {
        let c = input.get(n).copied().unwrap_or(0);
        if c == 0 {
            break;
        }
        out[n] = c;
        n += 1;
    }
    out[n..out_max - 1].fill(0);
}

const NSFE_INFO_SIZE: i32 = 16;

/// The default NSF header
fn base_header() -> [u8; HEADER_SIZE] {
    let mut h = [0u8; HEADER_SIZE];
    h[..5].copy_from_slice(b"NESM\x1A"); // tag
    h[header::VERS] = 1; // version
    h[header::TRACK_COUNT] = 1; // track count
    h[header::FIRST_TRACK] = 1; // first track
                                // addresses, strings: 0
    h[header::NTSC_SPEED] = 0x1A; // NTSC rate
    h[header::NTSC_SPEED + 1] = 0x41;
    // banks: 0
    h[header::PAL_SPEED] = 0x20; // PAL rate
    h[header::PAL_SPEED + 1] = 0x4E;
    // flags, unused: 0
    h
}

/// `Nsfe_Info::load()`
pub fn nsfe_info_load<H: NsfeInfoHost + ?Sized>(
    h: &mut H,
    input: &mut dyn DataReader,
) -> BlarggErr {
    // check header
    let mut signature = [0u8; 4];
    if let Err(err) = input.read(&mut signature) {
        return Err(if err == EOF_ERROR {
            GME_WRONG_FILE_TYPE
        } else {
            err
        });
    }
    if &signature != b"NSFE" {
        return Err(GME_WRONG_FILE_TYPE);
    }

    // free previous info
    {
        let info = h.nsfe_info();
        info.track_name_data = Vec::new();
        info.track_names = Vec::new();
        info.playlist = Vec::new();
        info.track_times = Vec::new();
        // FIXME (upstream): the strings are left as they were (uninitialized
        // at first) where there is no auth block; they are cleared here, as
        // in upstream's patched build for the tests.
        info.info.game = [0; 256];
        info.info.author = [0; 256];
        info.info.copyright = [0; 256];
        info.info.dumper = [0; 256];

        // default nsf header
        info.info.header = base_header();
    }

    // parse tags
    let mut phase = 0;
    while phase != 3 {
        // read size and tag
        let mut block_header = [0u8; 8];
        input.read(&mut block_header)?;
        let size = get_le32(&block_header[0..4]) as i32;
        let tag = get_le32(&block_header[4..8]) as i32 as i64;

        if size < 0 {
            return Err("Corrupt file");
        }

        //debug_printf( "tag: %c%c%c%c\n", char(tag), char(tag>>8), char(tag>>16), char(tag>>24) );

        if tag == blargg_4char(b'O', b'F', b'N', b'I') {
            // check( phase == 0 );
            if size < 8 {
                return Err("Corrupt file");
            }

            let mut finfo = [0u8; NSFE_INFO_SIZE as usize];
            finfo[8] = 1; // track_count
            finfo[9] = 0; // first_track

            input.read(&mut finfo[..size.min(NSFE_INFO_SIZE) as usize])?;
            if size > NSFE_INFO_SIZE {
                input.skip((size - NSFE_INFO_SIZE) as i64)?;
            }
            phase = 1;
            let info = h.nsfe_info();
            let hd = &mut info.info.header;
            hd[header::SPEED_FLAGS] = finfo[6];
            hd[header::CHIP_FLAGS] = finfo[7];
            hd[header::TRACK_COUNT] = finfo[8];
            info.actual_track_count_ = finfo[8] as i32;
            info.info.header[header::FIRST_TRACK] = finfo[9];
            info.info.header[header::LOAD_ADDR..header::LOAD_ADDR + 6].copy_from_slice(&finfo[..6]);
        } else if tag == blargg_4char(b'K', b'N', b'A', b'B') {
            if size > 8 {
                return Err("Corrupt file");
            }
            input.read(
                &mut h.nsfe_info().info.header[header::BANKS..header::BANKS + size as usize],
            )?;
        } else if tag == blargg_4char(b'h', b't', b'u', b'a') {
            let mut chars = Vec::new();
            let mut strs = Vec::new();
            read_strs(input, size as i64, &mut chars, &mut strs)?;
            let n = strs.len();
            let info = &mut h.nsfe_info().info;

            if n > 3 {
                copy_str(&chars[strs[3]..], &mut info.dumper);
            }

            if n > 2 {
                copy_str(&chars[strs[2]..], &mut info.copyright);
            }

            if n > 1 {
                copy_str(&chars[strs[1]..], &mut info.author);
            }

            if n > 0 {
                copy_str(&chars[strs[0]..], &mut info.game);
            }
        } else if tag == blargg_4char(b'e', b'm', b'i', b't') {
            check_remain(input, (size / 4) as i64 * 4)?;
            let info = h.nsfe_info();
            try_resize(&mut info.track_times, (size / 4) as usize)?;
            let n = info.track_times.len() * 4;
            let mut bytes = vec![0u8; n];
            let r = input.read(&mut bytes);
            for (t, c) in info.track_times.iter_mut().zip(bytes.chunks_exact(4)) {
                t.copy_from_slice(c);
            }
            r?;
        } else if tag == blargg_4char(b'l', b'b', b'l', b't') {
            let info = h.nsfe_info();
            read_strs(
                input,
                size as i64,
                &mut info.track_name_data,
                &mut info.track_names,
            )?;
        } else if tag == blargg_4char(b't', b's', b'l', b'p') {
            check_remain(input, size as i64)?;
            let info = h.nsfe_info();
            try_resize(&mut info.playlist, size as usize)?;
            input.read(&mut info.playlist[..])?;
        } else if tag == blargg_4char(b'A', b'T', b'A', b'D') {
            // check( phase == 1 );
            phase = 2;
            match h.load_nsf(input, size as i64) {
                None => input.skip(size as i64)?,
                Some(r) => r?,
            }
        } else if tag == blargg_4char(b'D', b'N', b'E', b'N') {
            // check( phase == 2 );
            // FIXME (upstream): an emulator loading a file without a DATA
            // block went on to play from memory never set up; this fails
            // the load then, as upstream's patched build for the tests does.
            if phase != 2 && h.load_nsf_wanted() {
                return Err("Corrupt file");
            }
            phase = 3;
        } else {
            // tags that can be skipped start with a lowercase character
            // check( islower( (tag >> 24) & 0xFF ) );
            input.skip(size as i64)?;
        }
    }

    Ok(())
}

impl NsfeInfo {
    fn unload(&mut self) {
        self.track_name_data = Vec::new();
        self.track_names = Vec::new();
        self.playlist = Vec::new();
        self.track_times = Vec::new();
    }

    // TODO: if no playlist, treat as if there is a playlist that is just 1,2,3,4,5... ?
    pub fn disable_playlist(&mut self, b: bool) {
        self.playlist_disabled = b;
        self.info.header[header::TRACK_COUNT] = self.playlist.len() as u8;
        if self.info.header[header::TRACK_COUNT] == 0 || self.playlist_disabled {
            self.info.header[header::TRACK_COUNT] = self.actual_track_count_ as u8;
        }
    }

    pub fn remap_track(&self, mut track: i32) -> i32 {
        if !self.playlist_disabled && (track as u32 as usize) < self.playlist.len() {
            track = self.playlist[track as usize] as i32;
        }
        track
    }

    pub fn track_info_(&self, out: &mut TrackInfo, track: i32) -> BlarggErr {
        let remapped = self.remap_track(track);
        if (remapped as u32 as usize) < self.track_times.len() {
            let length = get_le32(&self.track_times[remapped as usize]) as i32 as i64;
            if length > 0 {
                out.length = length;
            }
        }
        if (remapped as u32 as usize) < self.track_names.len() {
            copy_field_(
                &mut out.song,
                &self.track_name_data[self.track_names[remapped as usize]..],
            );
        }

        gme_copy_field(&mut out.game, &self.info.game);
        gme_copy_field(&mut out.author, &self.info.author);
        gme_copy_field(&mut out.copyright, &self.info.copyright);
        gme_copy_field(&mut out.dumper, &self.info.dumper);
        Ok(())
    }
}

/// `Nsfe_Emu`
pub struct NsfeEmu {
    nsf: NsfEmu,
    info: NsfeInfo,
    loading: bool,
}

impl std::fmt::Debug for NsfeEmu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NsfeEmu").finish_non_exhaustive()
    }
}

impl Default for NsfeEmu {
    fn default() -> Self {
        Self::new()
    }
}

impl NsfeEmu {
    pub fn new() -> Self {
        let mut nsf = NsfEmu::new();
        nsf.state.set_type(&GME_NSFE_TYPE_);
        NsfeEmu {
            nsf,
            info: NsfeInfo::default(),
            loading: false,
        }
    }

    /// use clear_playlist()
    pub fn disable_playlist(&mut self, b: bool) {
        self.info.disable_playlist(b);
        let n = self.info.info.header[header::TRACK_COUNT] as i32;
        self.nsf.state.set_track_count(n);
    }
}

impl NsfeInfoHost for NsfeEmu {
    fn nsfe_info(&mut self) -> &mut NsfeInfo {
        &mut self.info
    }

    fn load_nsf(&mut self, input: &mut dyn DataReader, size: i64) -> Option<BlarggErr> {
        let mut sub = SubsetReader::new(input, size); // limit emu to nsf data
                                                      // nsf_emu->load( rem ): Gme_File::load(), with the Remaining_Reader
                                                      // reading the header as it is after pre_load() (upstream reads it
                                                      // through a pointer to it once loading starts)
        self.pre_load();
        let header = self.info.info.header;
        let mut rem = RemainingReader::new(&header, &mut sub);
        let err = self.load_(&mut rem);
        // check( rem.remain() == 0 );
        Some(gme_file::post_load(self, err))
    }

    fn load_nsf_wanted(&self) -> bool {
        true
    }
}

/// `Nsfe_File`: the info-only NSFE reader
struct NsfeFile {
    state: MusicEmuState,
    info: NsfeInfo,
}

impl NsfeInfoHost for NsfeFile {
    fn nsfe_info(&mut self) -> &mut NsfeInfo {
        &mut self.info
    }

    fn load_nsf(&mut self, _input: &mut dyn DataReader, _size: i64) -> Option<BlarggErr> {
        None
    }

    fn load_nsf_wanted(&self) -> bool {
        false
    }
}

impl MusicEmu for NsfeFile {
    gme_info_overrides!();

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        nsfe_info_load(self, input)?;
        self.info.disable_playlist(false);
        let n = self.info.info.header[header::TRACK_COUNT] as i32;
        self.state.set_track_count(n);
        Ok(())
    }

    fn track_info_(&self, out: &mut TrackInfo, track: i32) -> BlarggErr {
        self.info.track_info_(out, track)
    }
}

fn new_nsfe_emu() -> Box<dyn MusicEmu> {
    Box::new(NsfeEmu::new())
}
fn new_nsfe_file() -> Box<dyn MusicEmu> {
    let mut state = MusicEmuState::new();
    state.set_type(&GME_NSFE_TYPE_);
    Box::new(NsfeFile {
        state,
        info: NsfeInfo::default(),
    })
}

pub(crate) static GME_NSFE_TYPE_: GmeTypeT = GmeTypeT {
    system: "Nintendo NES",
    track_count: 0,
    new_emu: new_nsfe_emu,
    new_info: new_nsfe_file,
    extension_: "NSFE",
    flags_: 1,
};

impl ClassicEmu for NsfeEmu {
    fn classic(&self) -> &ClassicEmuState {
        &self.nsf.classic
    }
    fn classic_mut(&mut self) -> &mut ClassicEmuState {
        &mut self.nsf.classic
    }
    fn set_voice(&mut self, i: i32, c: Option<BufId>, l: Option<BufId>, r: Option<BufId>) {
        self.nsf.set_voice(i, c, l, r);
    }
    fn update_eq(&mut self, eq: &BlipEq) {
        self.nsf.update_eq(eq);
    }
    fn run_clocks(&mut self, time_io: &mut BlipTime, msec: i32) -> BlarggErr {
        self.nsf.run_clocks(time_io, msec)
    }
}

impl MusicEmu for NsfeEmu {
    fn emu(&self) -> &MusicEmuState {
        &self.nsf.state
    }
    fn emu_mut(&mut self) -> &mut MusicEmuState {
        &mut self.nsf.state
    }
    fn set_buffer(&mut self, buf: Box<dyn crate::multi_buffer::MultiBuffer>) {
        self.nsf.set_buffer(buf);
    }
    fn effects_buffer(&mut self) -> Option<&mut crate::effects_buffer::EffectsBuffer> {
        self.nsf.effects_buffer()
    }
    fn set_multi_channel(&mut self, is_enabled: bool) -> BlarggErr {
        crate::classic_emu::classic_set_multi_channel(self, is_enabled)
    }
    fn set_sample_rate_(&mut self, rate: i64) -> BlarggErr {
        crate::classic_emu::classic_set_sample_rate_(self, rate)
    }
    fn mute_voices_(&mut self, mask: i32) {
        crate::classic_emu::classic_mute_voices_(self, mask)
    }
    fn set_equalizer_(&mut self, eq: &Equalizer) {
        crate::classic_emu::classic_set_equalizer_(self, eq)
    }
    fn play_(&mut self, out: &mut [Sample]) -> BlarggErr {
        crate::classic_emu::classic_play_(self, out)
    }
    fn set_tempo_(&mut self, t: f64) {
        self.nsf.set_tempo_(t);
    }

    fn unload(&mut self) {
        if !self.loading {
            self.info.unload(); // TODO: extremely hacky!
        }
        // Nsf_Emu::unload()
        self.nsf.nsf_unload();
        music_emu_unload(self);
    }

    fn track_info_(&self, out: &mut TrackInfo, track: i32) -> BlarggErr {
        self.info.track_info_(out, track)
    }

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        if self.loading {
            return self.nsf.nsf_load_(input);
        }

        // TODO: this hacky recursion-avoidance could have subtle problems
        self.loading = true;
        let err = nsfe_info_load(self, input);
        self.loading = false;
        self.disable_playlist(false);
        err
    }

    fn clear_playlist_(&mut self) {
        self.disable_playlist(true);
        // Nsf_Emu::clear_playlist_(): nothing
    }

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        let t = self.info.remap_track(track);
        self.nsf.nsf_start_track_(t)
    }
}
