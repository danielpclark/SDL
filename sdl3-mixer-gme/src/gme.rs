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

//! The C interface of Game_Music_Emu (`gme.h`, `gme.cpp`), on
//! [`MusicEmu`] trait objects (upstream's `Music_Emu*`), with upstream's
//! `int` arguments and results. File and M3U loading by path, the custom
//! reader and `gme_load_tracks()` are left out (SDL_mixer loads from
//! memory), as are the user data functions.

use crate::blargg_common::{blargg_2char, blargg_4char, cstr, get_be16, get_be32, BlarggErr};
use crate::data_reader::MemFileReader;
use crate::effects_buffer::EffectsBuffer;
use crate::gme_file::{GmeType, TrackInfo, GME_WRONG_FILE_TYPE};
use crate::music_emu::{Equalizer, MusicEmu};

/// `gme_err_t`: `Ok` on success, otherwise the error string
pub type GmeErr = BlarggErr;

/// Version of the library (`GME_VERSION`: 1 byte major, 1 byte minor, 1
/// byte patch-level)
pub const GME_VERSION: i32 = 0x000605;

/// Pass as `sample_rate` to open/load a file for its information only
pub const GME_INFO_ONLY: i32 = -1;

pub use crate::gme_file::GME_WRONG_FILE_TYPE as gme_wrong_file_type;

/// `gme_type_list()`: the music types the library supports, in upstream's
/// order (`USE_GME_*` all on, as SDL_mixer's CMake builds it).
pub fn gme_type_list() -> &'static [GmeType] {
    static LIST: [GmeType; 3] = [
        &crate::gbs_emu::GME_GBS_TYPE_,
        &crate::nsf_emu::GME_NSF_TYPE_,
        &crate::nsfe_emu::GME_NSFE_TYPE_,
    ];
    &LIST
}

pub static GME_GBS_TYPE: GmeType = &crate::gbs_emu::GME_GBS_TYPE_;
pub static GME_NSF_TYPE: GmeType = &crate::nsf_emu::GME_NSF_TYPE_;
pub static GME_NSFE_TYPE: GmeType = &crate::nsfe_emu::GME_NSFE_TYPE_;

/// `gme_identify_header()`: the file extension ("NSF", "SPC"...) its first
/// four bytes say a file has, or "" if they don't say
pub fn gme_identify_header(header: &[u8]) -> &'static str {
    let v = get_be32(header) as i64;
    let tags: [(i64, &'static str); 12] = [
        (blargg_4char(b'Z', b'X', b'A', b'Y'), "AY"),
        (blargg_4char(b'G', b'B', b'S', 0x01), "GBS"),
        (blargg_4char(b'G', b'Y', b'M', b'X'), "GYM"),
        (blargg_4char(b'H', b'E', b'S', b'M'), "HES"),
        (blargg_4char(b'K', b'S', b'C', b'C'), "KSS"),
        (blargg_4char(b'K', b'S', b'S', b'X'), "KSS"),
        (blargg_4char(b'N', b'E', b'S', b'M'), "NSF"),
        (blargg_4char(b'N', b'S', b'F', b'E'), "NSFE"),
        (blargg_4char(b'S', b'A', b'P', 0x0D), "SAP"),
        (blargg_4char(b'S', b'N', b'E', b'S'), "SPC"),
        (blargg_4char(b'V', b'g', b'm', b' '), "VGM"),
        (-1, ""),
    ];
    for (tag, ext) in tags {
        if tag == v {
            return ext;
        }
    }
    if get_be16(header) as i64 == blargg_2char(0x1F, 0x8B) {
        return "VGZ";
    }
    ""
}

/// `gme_identify_extension()`: the type with the extension of a path or an
/// extension, if one has it
pub fn gme_identify_extension(extension_: &[u8]) -> Option<GmeType> {
    let extension_ = cstr(extension_);
    let ext = match extension_.iter().rposition(|&c| c == b'.') {
        Some(n) => &extension_[n + 1..],
        None => extension_,
    };

    // to_uppercase( extension_, sizeof extension, extension );
    let mut extension = [0u8; 6];
    let mut terminated = false;
    for i in 0..6 {
        let c = ext.get(i).copied().unwrap_or(0);
        extension[i] = c.to_ascii_uppercase();
        if extension[i] == 0 {
            terminated = true;
            break;
        }
    }
    if !terminated {
        extension[0] = 0; // extension too long
    }

    gme_type_list()
        .iter()
        .find(|t| cstr(&extension) == t.extension_.as_bytes())
        .copied()
}

/// `gme_type_extension()`
pub fn gme_type_extension(music_type: Option<GmeType>) -> &'static str {
    match music_type {
        Some(t) => t.extension_,
        None => "",
    }
}

/// `gme_open_data()`: create an emulator for the data, at `sample_rate`
/// (or for its information only, with [`GME_INFO_ONLY`]), and load it
pub fn gme_open_data(data: &[u8], sample_rate: i32) -> Result<Box<dyn MusicEmu>, &'static str> {
    let mut file_type = None;
    if data.len() >= 4 {
        file_type = gme_identify_extension(gme_identify_header(data).as_bytes());
    }
    let Some(file_type) = file_type else {
        return Err(GME_WRONG_FILE_TYPE);
    };

    let Some(mut emu) = gme_new_emu(file_type, sample_rate) else {
        return Err("Out of memory");
    };

    gme_load_data(emu.as_mut(), data)?;
    Ok(emu)
}

/// `gme_set_autoload_playback_limit()`
pub fn gme_set_autoload_playback_limit(emu: &mut dyn MusicEmu, do_autoload_limit: i32) {
    emu.set_autoload_playback_limit(do_autoload_limit != 0);
}

/// `gme_autoload_playback_limit()`
pub fn gme_autoload_playback_limit(emu: &dyn MusicEmu) -> i32 {
    emu.autoload_playback_limit() as i32
}

/// Used to implement gme_new_emu and gme_new_emu_multi_channel
fn gme_internal_new_emu_(
    type_: GmeType,
    rate: i32,
    multi_channel: bool,
) -> Option<Box<dyn MusicEmu>> {
    if rate == GME_INFO_ONLY {
        return Some((type_.new_info)());
    }

    let mut me = (type_.new_emu)();
    let _ = me.set_multi_channel(multi_channel);

    if type_.flags_ & 1 != 0 {
        let effects_buffer = if me.multi_channel() {
            EffectsBuffer::new(8, false)
        } else {
            EffectsBuffer::new(1, false)
        };
        me.set_buffer(Box::new(effects_buffer));
    }

    if me.set_sample_rate(rate as i64).is_ok() {
        // check( me->type() == type );
        return Some(me);
    }
    None
}

/// `gme_new_emu()`: create an emulator of the type at `rate` (or for its
/// information only, with [`GME_INFO_ONLY`]), or `None` if out of memory
pub fn gme_new_emu(type_: GmeType, rate: i32) -> Option<Box<dyn MusicEmu>> {
    gme_internal_new_emu_(type_, rate, false /* no multichannel */)
}

/// `gme_new_emu_multi_channel()`: likewise, rendering each voice to its
/// own stereo pair, where the type can
pub fn gme_new_emu_multi_channel(type_: GmeType, rate: i32) -> Option<Box<dyn MusicEmu>> {
    // multi-channel emulator (if possible, not all emu types support multi-channel)
    gme_internal_new_emu_(type_, rate, true /* multichannel */)
}

/// `gme_load_data()`
pub fn gme_load_data(me: &mut dyn MusicEmu, data: &[u8]) -> GmeErr {
    let mut input = MemFileReader::new(data);
    me.load(&mut input)
}

/// `gme_type()`
pub fn gme_type(me: &dyn MusicEmu) -> GmeType {
    me.type_()
}

/// `gme_warning()`
pub fn gme_warning(me: &mut dyn MusicEmu) -> Option<&'static str> {
    me.warning()
}

/// `gme_track_count()`
pub fn gme_track_count(me: &dyn MusicEmu) -> i32 {
    me.track_count()
}

/// `gme_info_t`: a track's information. The strings are the bytes of the
/// C strings (without their terminators).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GmeInfo {
    /// times in milliseconds; -1 if unknown
    /// total length, if file specifies it
    pub length: i32,
    /// length of song up to looping section
    pub intro_length: i32,
    /// length of looping section
    pub loop_length: i32,
    /// Length if available, otherwise intro_length+loop_length*2 if available,
    /// otherwise a default of 150000 (2.5 minutes).
    pub play_length: i32,
    /// fade length in milliseconds; -1 if unknown
    pub fade_length: i32,

    // empty string ("") if not available
    pub system: Vec<u8>,
    pub game: Vec<u8>,
    pub song: Vec<u8>,
    pub author: Vec<u8>,
    pub copyright: Vec<u8>,
    pub comment: Vec<u8>,
    pub dumper: Vec<u8>,
}

/// `gme_track_info()` (and `gme_free_info()`, the info's `Drop`)
pub fn gme_track_info(me: &dyn MusicEmu, track: i32) -> Result<GmeInfo, &'static str> {
    let mut info = Box::<TrackInfo>::default();
    me.track_info(&mut info, track)?;

    let s = |f: &[u8; 256]| cstr(f).to_vec();
    let mut out = GmeInfo {
        length: info.length as i32,
        intro_length: info.intro_length as i32,
        loop_length: info.loop_length as i32,
        fade_length: info.fade_length as i32,
        play_length: 0,
        system: s(&info.system),
        game: s(&info.game),
        song: s(&info.song),
        author: s(&info.author),
        copyright: s(&info.copyright),
        comment: s(&info.comment),
        dumper: s(&info.dumper),
    };

    out.play_length = out.length;
    if out.play_length <= 0 {
        out.play_length = out
            .intro_length
            .wrapping_add(out.loop_length.wrapping_mul(2)); // intro + 2 loops
        if out.play_length <= 0 {
            out.play_length = 150 * 1000; // 2.5 minutes
        }
    }

    Ok(out)
}

/// `gme_set_stereo_depth()`: adjust stereo echo depth, where 0.0 = off and
/// 1.0 = maximum (has no effect for GYM, SPC, and Sega Genesis VGM music)
pub fn gme_set_stereo_depth(me: &mut dyn MusicEmu, depth: f64) {
    if let Some(eb) = me.effects_buffer() {
        eb.set_depth(depth);
    }
}

/// `gme_start_track()`
pub fn gme_start_track(me: &mut dyn MusicEmu, index: i32) -> GmeErr {
    me.start_track(index)
}
/// `gme_play()`: generate `out.len()` samples (stereo pairs, so an even
/// count)
pub fn gme_play(me: &mut dyn MusicEmu, out: &mut [i16]) -> GmeErr {
    me.play(out)
}
/// `gme_set_fade()`
pub fn gme_set_fade(me: &mut dyn MusicEmu, start_msec: i32) {
    me.set_fade(start_msec as i64, 8000);
}
/// `gme_set_fade_msecs()`
pub fn gme_set_fade_msecs(me: &mut dyn MusicEmu, start_msec: i32, fade_msec: i32) {
    me.set_fade(start_msec as i64, fade_msec as i64);
}
/// `gme_track_ended()`
pub fn gme_track_ended(me: &dyn MusicEmu) -> i32 {
    me.track_ended() as i32
}
/// `gme_tell()`
pub fn gme_tell(me: &dyn MusicEmu) -> i32 {
    me.tell() as i32
}
/// `gme_tell_samples()`
pub fn gme_tell_samples(me: &dyn MusicEmu) -> i32 {
    me.tell_samples() as i32
}
/// `gme_tell_scaled()`
pub fn gme_tell_scaled(me: &dyn MusicEmu) -> i32 {
    me.tell_scaled() as i32
}
/// `gme_seek()`
pub fn gme_seek(me: &mut dyn MusicEmu, msec: i32) -> GmeErr {
    me.seek(msec as i64)
}
/// `gme_seek_samples()`
pub fn gme_seek_samples(me: &mut dyn MusicEmu, n: i32) -> GmeErr {
    me.seek_samples(n as i64)
}
/// `gme_seek_scaled()`
pub fn gme_seek_scaled(me: &mut dyn MusicEmu, msec: i32) -> GmeErr {
    me.seek_scaled(msec as i64)
}
/// `gme_voice_count()`
pub fn gme_voice_count(me: &dyn MusicEmu) -> i32 {
    me.voice_count()
}
/// `gme_ignore_silence()`
pub fn gme_ignore_silence(me: &mut dyn MusicEmu, disable: i32) {
    me.ignore_silence(disable != 0);
}
/// `gme_set_tempo()`
pub fn gme_set_tempo(me: &mut dyn MusicEmu, t: f64) {
    me.set_tempo(t);
}
/// `gme_mute_voice()`
pub fn gme_mute_voice(me: &mut dyn MusicEmu, index: i32, mute: i32) {
    me.mute_voice(index, mute != 0);
}
/// `gme_mute_voices()`
pub fn gme_mute_voices(me: &mut dyn MusicEmu, mask: i32) {
    me.mute_voices(mask);
}
/// `gme_disable_echo()`
pub fn gme_disable_echo(me: &mut dyn MusicEmu, disable: i32) {
    me.disable_echo(disable != 0);
}
/// `gme_enable_accuracy()`
pub fn gme_enable_accuracy(me: &mut dyn MusicEmu, enabled: i32) {
    me.enable_accuracy(enabled != 0);
}
/// `gme_clear_playlist()`
pub fn gme_clear_playlist(me: &mut dyn MusicEmu) {
    me.clear_playlist();
}
/// `gme_type_multitrack()`
pub fn gme_type_multitrack(t: GmeType) -> i32 {
    (t.track_count != 1) as i32
}
/// `gme_multi_channel()`
pub fn gme_multi_channel(me: &dyn MusicEmu) -> i32 {
    me.multi_channel() as i32
}

/// `gme_set_equalizer()`
pub fn gme_set_equalizer(me: &mut dyn MusicEmu, eq: &Equalizer) {
    let mut e = me.equalizer();
    e.treble = eq.treble;
    e.bass = eq.bass;
    me.set_equalizer(&e);
}

/// `gme_equalizer()`
pub fn gme_equalizer(me: &dyn MusicEmu) -> Equalizer {
    let e = me.equalizer();
    Equalizer {
        treble: e.treble,
        bass: e.bass,
    }
}

/// `gme_voice_name()`
pub fn gme_voice_name(me: &dyn MusicEmu, i: i32) -> &'static str {
    // assert( (unsigned) i < (unsigned) me->voice_count() );
    me.voice_names()[i as usize]
}

/// `gme_type_system()`
pub fn gme_type_system(type_: GmeType) -> &'static str {
    type_.system
}

/// `gme_fixed_track_count()`
pub fn gme_fixed_track_count(t: GmeType) -> i32 {
    t.track_count
}
