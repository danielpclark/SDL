// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2003-2006 Shay Green. This module is free software; you
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

//! Common interface to game music file loading and information
//! (`Gme_File.h`, `Gme_File.cpp`). The `Gme_File` virtuals are in the
//! [`MusicEmu`] trait with `Music_Emu`'s; these are its non-virtual
//! functions and the default implementations of its virtual ones.
//!
//! M3U playlists (`M3u_Playlist`, `load_m3u()`) are left out: SDL_mixer
//! never loads one, so the playlist stays empty, as it is here.

use crate::blargg_common::{try_resize, BlarggErr};
use crate::data_reader::{DataReader, MemFileReader};
use crate::music_emu::MusicEmu;

/// Error returned if file is wrong type
pub const GME_WRONG_FILE_TYPE: &str = "Wrong file type for this emulator";

/// `gme_type_t_`: a music file type.
pub struct GmeTypeT {
    /// name of system this music file type is generally for
    pub system: &'static str,
    /// non-zero for formats with a fixed number of tracks
    pub track_count: i32,
    /// Create new emulator for this type (useful in C++ only)
    pub new_emu: fn() -> Box<dyn MusicEmu>,
    /// Create new info reader for this type
    pub new_info: fn() -> Box<dyn MusicEmu>,

    // internal
    pub extension_: &'static str,
    pub flags_: i32,
}

impl std::fmt::Debug for GmeTypeT {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GmeTypeT")
            .field("system", &self.system)
            .field("extension", &self.extension_)
            .finish_non_exhaustive()
    }
}

/// `gme_type_t`
pub type GmeType = &'static GmeTypeT;

pub const GME_MAX_FIELD: usize = 255;

/// `track_info_t`. The strings are C strings in their 256-byte arrays.
#[derive(Clone)]
pub struct TrackInfo {
    pub track_count: i64,

    /// times in milliseconds; -1 if unknown
    pub length: i64,
    pub intro_length: i64,
    pub loop_length: i64,
    pub fade_length: i64,
    pub repeat_count: i64,

    /// Length if available, otherwise intro_length+loop_length*2 if available,
    /// otherwise a default of 150000 (2.5 minutes)
    pub play_length: i64,

    /// empty string if not available
    pub system: [u8; 256],
    pub game: [u8; 256],
    pub song: [u8; 256],
    pub author: [u8; 256],
    pub composer: [u8; 256],
    pub engineer: [u8; 256],
    pub sequencer: [u8; 256],
    pub tagger: [u8; 256],
    pub copyright: [u8; 256],
    pub date: [u8; 256],
    pub comment: [u8; 256],
    pub dumper: [u8; 256],
    pub disc: [u8; 256],
    pub track: [u8; 256],
    pub ost: [u8; 256],
}

impl Default for TrackInfo {
    fn default() -> Self {
        TrackInfo {
            track_count: 0,
            length: 0,
            intro_length: 0,
            loop_length: 0,
            fade_length: 0,
            repeat_count: 0,
            play_length: 0,
            system: [0; 256],
            game: [0; 256],
            song: [0; 256],
            author: [0; 256],
            composer: [0; 256],
            engineer: [0; 256],
            sequencer: [0; 256],
            tagger: [0; 256],
            copyright: [0; 256],
            date: [0; 256],
            comment: [0; 256],
            dumper: [0; 256],
            disc: [0; 256],
            track: [0; 256],
            ost: [0; 256],
        }
    }
}

impl std::fmt::Debug for TrackInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackInfo")
            .field("track_count", &self.track_count)
            .field("length", &self.length)
            .field(
                "song",
                &String::from_utf8_lossy(crate::blargg_common::cstr(&self.song)),
            )
            .finish_non_exhaustive()
    }
}

/// The fields of `Gme_File`.
#[derive(Debug, Default)]
pub struct GmeFileState {
    pub(crate) type_: Option<GmeType>,
    pub(crate) track_count_: i32,
    pub(crate) raw_track_count_: i32,
    pub(crate) warning_: Option<&'static str>,
    /// only if loaded into memory using default load
    pub(crate) file_data: Vec<u8>,
    /// file start indexes of `file_data`
    pub(crate) tracks: Vec<i64>,
}

impl GmeFileState {
    pub(crate) fn set_track_count(&mut self, n: i32) {
        self.track_count_ = n;
        self.raw_track_count_ = n;
    }

    pub(crate) fn set_warning(&mut self, s: &'static str) {
        self.warning_ = Some(s);
    }

    /// (`track_pos()`, as an index into `file_data`)
    pub(crate) fn track_pos(&self, i: i32) -> usize {
        self.tracks[i as usize] as usize
    }

    pub(crate) fn track_size(&self, i: i32) -> usize {
        (self.tracks[i as usize + 1] - self.tracks[i as usize]) as usize
    }

    /// `type()`
    pub(crate) fn type_(&self) -> GmeType {
        self.type_.expect("type set by the constructor")
    }

    pub(crate) fn track_count(&self) -> i32 {
        self.track_count_
    }

    /// `remap_track_()`
    pub fn remap_track_(&self, track_io: &mut i32) -> BlarggErr {
        if *track_io as u32 >= self.track_count() as u32 {
            return Err("Invalid track");
        }

        // (the m3u playlist is always empty here)
        Ok(())
    }
}

/// `Gme_File::clear_playlist()`
pub fn clear_playlist<E: MusicEmu + ?Sized>(this: &mut E) {
    this.clear_playlist_();
    let f = &mut this.emu_mut().file;
    f.track_count_ = f.raw_track_count_;
}

/// `Gme_File::unload()`
pub fn gme_file_unload<E: MusicEmu + ?Sized>(this: &mut E) {
    clear_playlist(this); // *before* clearing track count
    let f = &mut this.emu_mut().file;
    f.track_count_ = 0;
    f.raw_track_count_ = 0;
    f.file_data = Vec::new();
}

/// `Gme_File::load_mem_()` (on `len` bytes of the file data at `pos`)
pub fn gme_file_load_mem_<E: MusicEmu + ?Sized>(this: &mut E, pos: usize, len: usize) -> BlarggErr {
    // require( data != file_data.begin() ); // load_mem_() or load_() must be overridden
    let data = this.emu().file.file_data[pos..pos + len].to_vec();
    let mut input = MemFileReader::new(&data);
    this.load_(&mut input)
}

/// `Gme_File::load_()`
pub fn gme_file_load_<E: MusicEmu + ?Sized>(this: &mut E, input: &mut dyn DataReader) -> BlarggErr {
    let n = input.remain();
    {
        let f = &mut this.emu_mut().file;
        try_resize(&mut f.file_data, n.max(0) as usize)?;
        input.read(&mut f.file_data)?;
        if f.type_().track_count == 1 {
            f.tracks = vec![0, f.file_data.len() as i64];
        }
    }
    let size = this.emu().file.file_data.len();
    this.load_mem_(0, size)
}

/// `Gme_File::post_load()`: public load functions call this at end
pub fn post_load<E: MusicEmu + ?Sized>(this: &mut E, err: BlarggErr) -> BlarggErr {
    if this.emu().file.track_count() == 0 {
        let n = this.emu().file.type_().track_count;
        this.emu_mut().file.set_track_count(n);
    }
    if err.is_ok() {
        this.post_load_();
    } else {
        this.unload();
    }

    err
}

/// `Gme_File::load()`
pub fn load<E: MusicEmu + ?Sized>(this: &mut E, input: &mut dyn DataReader) -> BlarggErr {
    this.pre_load();
    let err = this.load_(input);
    post_load(this, err)
}

/// `Gme_File::copy_field_()`: copy a track info field, removing junk from
/// either end, truncated to 255 characters
pub fn copy_field_(out: &mut [u8], input: &[u8]) {
    copy_field_n(out, input, GME_MAX_FIELD as i32);
}

/// `Gme_File::copy_field_( out, in, in_size )`
pub fn copy_field_n(out: &mut [u8], input: &[u8], mut in_size: i32) {
    let mut input = input;
    if input.is_empty() || input[0] == 0 {
        return;
    }

    // remove spaces/junk from beginning
    while in_size != 0
        && !input.is_empty()
        && (input[0] as i8 as i32 - 1) as u32 <= (b' ' - 1) as u32
    {
        input = &input[1..];
        in_size -= 1;
    }

    // truncate
    if in_size > GME_MAX_FIELD as i32 {
        in_size = GME_MAX_FIELD as i32;
    }

    // find terminator
    let mut len = 0usize;
    while (len as i32) < in_size && len < input.len() && input[len] != 0 {
        len += 1;
    }

    // remove spaces/junk from end
    while len != 0 && input[len - 1] as i8 as i32 as u32 <= b' ' as u32 {
        len -= 1;
    }

    // copy
    out[len] = 0;
    out[..len].copy_from_slice(&input[..len]);

    // strip out stupid fields that should have been left blank
    let s = &out[..len];
    if s == b"?" || s == b"<?>" || s == b"< ? >" {
        out[0] = 0;
    }
}

/// `GME_COPY_FIELD( in, out, name )`: copy a header's fixed-size field
pub fn gme_copy_field(out: &mut [u8], input: &[u8]) {
    copy_field_n(out, input, input.len() as i32);
}

/// `Gme_File::track_info()`
pub fn track_info<E: MusicEmu + ?Sized>(this: &E, out: &mut TrackInfo, track: i32) -> BlarggErr {
    let f = &this.emu().file;
    out.track_count = f.track_count() as i64;
    out.length = -1;
    out.loop_length = -1;
    out.intro_length = -1;
    out.fade_length = -1;
    out.play_length = -1;
    out.repeat_count = -1;
    out.song[0] = 0;

    out.game[0] = 0;
    out.author[0] = 0;
    out.composer[0] = 0;
    out.engineer[0] = 0;
    out.sequencer[0] = 0;
    out.tagger[0] = 0;
    out.copyright[0] = 0;
    out.date[0] = 0;
    out.comment[0] = 0;
    out.dumper[0] = 0;
    out.system[0] = 0;
    out.disc[0] = 0;
    out.track[0] = 0;
    out.ost[0] = 0;

    copy_field_(&mut out.system, f.type_().system.as_bytes());

    let mut remapped = track;
    f.remap_track_(&mut remapped)?;
    this.track_info_(out, remapped)?;

    // (no m3u info to override with)
    Ok(())
}
