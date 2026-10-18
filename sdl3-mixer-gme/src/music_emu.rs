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

//! Common interface to game music file emulators (`Music_Emu.h`,
//! `Music_Emu.cpp`): the [`MusicEmu`] trait, whose required and
//! overridable methods are the virtual functions of `Gme_File` and
//! `Music_Emu`, and whose provided ones are their public functions.

use crate::blargg_common::{try_vec, BlarggErr};
use crate::data_reader::DataReader;
use crate::effects_buffer::EffectsBuffer;
use crate::gme_file::{self, GmeFileState, GmeType, TrackInfo};
use crate::multi_buffer::MultiBuffer;

/// `gme_equalizer_t` (`Music_Emu::equalizer_t`)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Equalizer {
    /// -50.0 = muffled, 0 = flat, +5.0 = extra-crisp
    pub treble: f64,
    /// 1 = full bass, 90 = average, 16000 = almost no bass
    pub bass: f64,
}

/// Construct equalizer of given treble/bass settings
pub const fn make_equalizer(treble: f64, bass: f64) -> Equalizer {
    Equalizer { treble, bass }
}

/// Equalizer settings for TV speaker
pub const TV_EQ: Equalizer = make_equalizer(-8.0, 180.0);

const SILENCE_MAX: i32 = 6; // seconds
const SILENCE_THRESHOLD: i32 = 0x10;
const FADE_BLOCK_SIZE: i64 = 512;
const FADE_SHIFT: i32 = 8; // fade ends with gain at 1.0 / (1 << fade_shift)

const BUF_SIZE: i64 = 2048;

pub type Sample = i16;

const DEFAULT_VOICE_NAMES: [&str; 8] = [
    "Voice 1", "Voice 2", "Voice 3", "Voice 4", "Voice 5", "Voice 6", "Voice 7", "Voice 8",
];

/// The fields of `Gme_File` and `Music_Emu`.
#[derive(Debug)]
pub struct MusicEmuState {
    pub(crate) file: GmeFileState,

    // general
    equalizer_: Equalizer,
    max_initial_silence: i32,
    voice_names_: Vec<&'static str>,
    voice_count_: i32,
    mute_mask_: i32,
    tempo_: f64,
    gain_: f64,
    multi_channel_: bool,

    sample_rate_: i64,

    // track-specific
    current_track_: i32,
    /// number of samples played since start of track
    out_time: i32,
    /// number of samples played since start of track (scaled with tempo)
    out_time_scaled: i32,
    /// number of samples emulator has generated since start of track
    emu_time: i32,
    /// emulator has reached end of track
    emu_track_ended_: bool,
    /// whether to load and obey track length by default
    emu_autoload_playback_limit_: bool,
    track_ended_: bool,

    // fading
    fade_start: i32,
    fade_step: i32,

    // silence detection
    /// speed to run emulator when looking ahead for silence
    silence_lookahead: i32,
    ignore_silence_: bool,
    /// number of samples where most recent silence began
    silence_time: i64,
    /// number of samples of silence to play before using buf
    silence_count: i64,
    /// number of samples left in silence buffer
    buf_remain: i64,
    buf: Vec<Sample>,
}

impl Default for MusicEmuState {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicEmuState {
    /// `Music_Emu::Music_Emu()` (and `Gme_File::Gme_File()`)
    pub fn new() -> Self {
        let mut s = MusicEmuState {
            file: GmeFileState::default(),
            equalizer_: Equalizer {
                treble: -1.0,
                bass: 60.0,
            },
            // defaults
            max_initial_silence: 2,
            voice_names_: DEFAULT_VOICE_NAMES.to_vec(),
            voice_count_: 0,
            mute_mask_: 0,
            tempo_: 1.0,
            gain_: 1.0,
            multi_channel_: false,
            sample_rate_: 0,
            current_track_: 0,
            out_time: 0,
            out_time_scaled: 0,
            emu_time: 0,
            emu_track_ended_: false,
            emu_autoload_playback_limit_: true,
            track_ended_: false,
            fade_start: 0,
            fade_step: 0,
            silence_lookahead: 3,
            ignore_silence_: false,
            silence_time: 0,
            silence_count: 0,
            buf_remain: 0,
            buf: Vec::new(),
        };
        // Music_Emu::unload(); // non-virtual
        s.voice_count_ = 0;
        s.clear_track_vars();
        // (Gme_File::unload(): the fields start cleared)
        s
    }

    fn clear_track_vars(&mut self) {
        self.current_track_ = -1;
        self.out_time = 0;
        self.out_time_scaled = 0;
        self.emu_time = 0;
        self.emu_track_ended_ = true;
        self.track_ended_ = true;
        self.fade_start = i32::MAX / 2 + 1;
        self.fade_step = 1;
        self.silence_time = 0;
        self.silence_count = 0;
        self.buf_remain = 0;
        self.file.warning_ = None; // clear warning
    }

    // Services for the emulators

    pub fn set_type(&mut self, t: GmeType) {
        self.file.type_ = Some(t);
    }
    pub fn set_warning(&mut self, s: &'static str) {
        self.file.set_warning(s);
    }
    pub fn set_track_count(&mut self, n: i32) {
        self.file.set_track_count(n);
    }
    pub fn set_max_initial_silence(&mut self, n: i32) {
        self.max_initial_silence = n;
    }
    pub fn set_silence_lookahead(&mut self, n: i32) {
        self.silence_lookahead = n;
    }
    pub fn set_voice_count(&mut self, n: i32) {
        self.voice_count_ = n;
    }
    pub fn set_voice_names(&mut self, names: &[&'static str]) {
        self.voice_names_ = names.to_vec();
    }
    pub fn set_track_ended(&mut self) {
        self.emu_track_ended_ = true;
    }
    pub fn gain(&self) -> f64 {
        self.gain_
    }
    pub fn tempo(&self) -> f64 {
        self.tempo_
    }
    pub fn mute_mask(&self) -> i32 {
        self.mute_mask_
    }
    pub fn sample_rate(&self) -> i64 {
        self.sample_rate_
    }
    pub fn voice_count(&self) -> i32 {
        self.voice_count_
    }
    pub fn equalizer(&self) -> Equalizer {
        self.equalizer_
    }
    pub fn emu_track_ended(&self) -> bool {
        self.emu_track_ended_
    }
    /// `track_ended()`
    pub fn track_ended(&self) -> bool {
        self.track_ended_
    }
    pub fn set_tempo_value(&mut self, t: f64) {
        self.tempo_ = t;
    }
    /// `Music_Emu::set_equalizer()`'s field, for constructors (which call
    /// the non-virtual function before the emulator is set up).
    pub fn set_equalizer_value(&mut self, eq: Equalizer) {
        self.equalizer_ = eq;
    }
    /// `Music_Emu::set_gain()`
    pub fn set_gain(&mut self, g: f64) {
        // assert( !sample_rate() ); // you must set gain before setting sample rate
        self.gain_ = g;
    }
    /// `set_multi_channel_()`
    pub fn set_multi_channel_(&mut self, is_enabled: bool) -> BlarggErr {
        // multi channel support must be set at the very beginning
        // require( !sample_rate() );
        self.multi_channel_ = is_enabled;
        Ok(())
    }

    /// returns the number of output channels, i.e. usually 2 for stereo, unlesss multi_channel_ == true
    fn out_channels(&self) -> i32 {
        if self.multi_channel_ {
            2 * 8
        } else {
            2
        }
    }

    fn msec_to_samples(&self, mut msec: i32) -> i32 {
        let sec = msec / 1000;
        msec -= sec * 1000;
        ((sec as i64)
            .wrapping_mul(self.sample_rate_)
            .wrapping_add((msec as i64).wrapping_mul(self.sample_rate_) / 1000))
        .wrapping_mul(self.out_channels() as i64) as i32
    }
}

/// Common interface to game music file emulators (`Gme_File` and
/// `Music_Emu`). (`Send`, so an emulator can play on SDL's audio thread.)
pub trait MusicEmu: Send {
    /// The fields of `Gme_File` and `Music_Emu`.
    fn emu(&self) -> &MusicEmuState;
    fn emu_mut(&mut self) -> &mut MusicEmuState;

    // Overridable (Gme_File)

    /// called before loading file and if loading fails
    fn unload(&mut self) {
        music_emu_unload(self);
    }

    /// default loads then calls load_mem_()
    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        gme_file::gme_file_load_(self, input)
    }

    /// use data in memory (`len` bytes of the file data at `pos`, which the
    /// emulator may keep referring to, as upstream keeps the pointer)
    fn load_mem_(&mut self, pos: usize, len: usize) -> BlarggErr {
        gme_file::gme_file_load_mem_(self, pos, len)
    }

    fn track_info_(&self, out: &mut TrackInfo, track: i32) -> BlarggErr;

    fn pre_load(&mut self) {
        music_emu_pre_load(self);
    }

    fn post_load_(&mut self) {
        music_emu_post_load_(self);
    }

    fn clear_playlist_(&mut self) {}

    // Overridable (Music_Emu)

    /// specifies if all 8 voices get rendered to their own stereo channel
    /// default implementation of Music_Emu always returns not supported error (i.e. no multichannel support by default)
    /// derived emus must override this if they support multichannel rendering
    fn set_multi_channel(&mut self, _is_enabled: bool) -> BlarggErr {
        // by default not supported, derived may override this
        Err("unsupported for this emulator type")
    }

    /// Request use of custom multichannel buffer. Only supported by "classic" emulators;
    /// on others this has no effect. Should be called only once *before* set_sample_rate().
    fn set_buffer(&mut self, _buf: Box<dyn MultiBuffer>) {}

    /// The `Effects_Buffer` `gme_internal_new_emu_()` gave the emulator,
    /// if it uses one (for `gme_set_stereo_depth()`).
    fn effects_buffer(&mut self) -> Option<&mut EffectsBuffer> {
        None
    }

    fn set_sample_rate_(&mut self, sample_rate: i64) -> BlarggErr;
    fn set_equalizer_(&mut self, _eq: &Equalizer) {}
    fn enable_accuracy_(&mut self, _enable: bool) {}
    fn mute_voices_(&mut self, _mask: i32) {}
    fn disable_echo_(&mut self, _disable: bool) {}
    fn set_tempo_(&mut self, t: f64) {
        self.emu_mut().tempo_ = t;
    }
    /// tempo is set before this
    fn start_track_(&mut self, track: i32) -> BlarggErr {
        music_emu_start_track_(self, track)
    }
    fn play_(&mut self, out: &mut [Sample]) -> BlarggErr;
    fn skip_(&mut self, count: i64) -> BlarggErr {
        music_emu_skip_(self, count)
    }

    // File loading (Gme_File)

    /// Load from custom data source (see Data_Reader.h)
    fn load(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        gme_file::load(self, input)
    }

    /// Clears any loaded m3u playlist and any internal playlist that the music
    /// format supports (NSFE for example).
    fn clear_playlist(&mut self) {
        gme_file::clear_playlist(self);
    }

    // Informational (Gme_File)

    /// Type of emulator. For example if this returns gme_nsfe_type, this object
    /// is an NSFE emulator.
    fn type_(&self) -> GmeType {
        self.emu().file.type_()
    }

    /// Most recent warning string, or NULL if none. Clears current warning after
    /// returning.
    fn warning(&mut self) -> Option<&'static str> {
        self.emu_mut().file.warning_.take()
    }

    /// Number of tracks or 0 if no file has been loaded
    fn track_count(&self) -> i32 {
        self.emu().file.track_count()
    }

    /// Get information for a track (length, name, author, etc.)
    fn track_info(&self, out: &mut TrackInfo, track: i32) -> BlarggErr {
        gme_file::track_info(self, out, track)
    }

    // Basic functionality (Music_Emu)

    /// Set output sample rate. Must be called only once before loading file.
    fn set_sample_rate(&mut self, rate: i64) -> BlarggErr {
        // require( !sample_rate() ); // sample rate can't be changed once set
        self.set_sample_rate_(rate)?;
        // Note (upstream): the buffer is left uninitialized (blargg_vector's
        // realloc), and when an emulator stops with an error before filling
        // it (as the SAP emulator does at an illegal instruction),
        // fill_buf() plays the rest of it; it starts zeroed here, and
        // upstream's build for the tests is patched the same.
        let buf = try_vec(BUF_SIZE as usize)?;
        let s = self.emu_mut();
        s.buf = buf;
        s.sample_rate_ = rate;
        Ok(())
    }

    /// Start a track, where 0 is the first track. Also clears warning string.
    fn start_track(&mut self, track: i32) -> BlarggErr {
        self.emu_mut().clear_track_vars();

        let mut remapped = track;
        self.emu().file.remap_track_(&mut remapped)?;
        self.emu_mut().current_track_ = track;
        self.start_track_(remapped)?;

        {
            let s = self.emu_mut();
            s.emu_track_ended_ = false;
            s.track_ended_ = false;
        }

        if !self.emu().ignore_silence_ {
            // play until non-silence or end of track
            let end = self.emu().max_initial_silence as i64
                * self.emu().out_channels() as i64
                * self.emu().sample_rate_;
            while (self.emu().emu_time as i64) < end {
                fill_buf(self);
                if self.emu().buf_remain != 0 || self.emu().emu_track_ended_ {
                    break;
                }
            }

            let s = self.emu_mut();
            s.emu_time = s.buf_remain as i32;
            s.out_time = 0;
            s.out_time_scaled = 0;
            s.silence_time = 0;
            s.silence_count = 0;
        }
        if self.track_ended() {
            match self.warning() {
                Some(w) => Err(w),
                None => Ok(()),
            }
        } else {
            Ok(())
        }
    }

    /// Generate 'count' samples info 'buf'. Output is in stereo. Any emulation
    /// errors set warning string, and major errors also end track.
    fn play(&mut self, out: &mut [Sample]) -> BlarggErr {
        let out_count = out.len() as i64;
        if self.emu().track_ended_ {
            out.fill(0);
        } else {
            // require( current_track() >= 0 );
            // require( out_count % out_channels() == 0 );

            // assert( emu_time >= out_time );

            // prints nifty graph of how far ahead we are when searching for silence
            //debug_printf( "%*s \n", int ((emu_time - out_time) * 7 / sample_rate()), "*" );

            let mut pos: i64 = 0;
            if self.emu().silence_count != 0 {
                // during a run of silence, run emulator at >=2x speed so it gets ahead
                let ahead_time = {
                    let s = self.emu();
                    s.silence_lookahead as i64 * (s.out_time as i64 + out_count - s.silence_time)
                        + s.silence_time
                };
                while (self.emu().emu_time as i64) < ahead_time
                    && !(self.emu().buf_remain != 0 || self.emu().emu_track_ended_)
                {
                    fill_buf(self);
                }

                // fill with silence
                let s = self.emu_mut();
                pos = s.silence_count.min(out_count);
                out[..pos as usize].fill(0);
                s.silence_count -= pos;

                if !s.ignore_silence_
                    && s.emu_time as i64 - s.silence_time
                        > SILENCE_MAX as i64 * s.out_channels() as i64 * s.sample_rate_
                {
                    s.track_ended_ = true;
                    s.emu_track_ended_ = true;
                    s.silence_count = 0;
                    s.buf_remain = 0;
                }
            }

            {
                let s = self.emu_mut();
                if s.buf_remain != 0 {
                    // empty silence buf
                    let n = s.buf_remain.min(out_count - pos);
                    let from = (BUF_SIZE - s.buf_remain) as usize;
                    out[pos as usize..(pos + n) as usize]
                        .copy_from_slice(&s.buf[from..from + n as usize]);
                    s.buf_remain -= n;
                    pos += n;
                }
            }

            // generate remaining samples normally
            let remain = out_count - pos;
            if remain != 0 {
                emu_play(self, &mut out[pos as usize..]);
                let s = self.emu_mut();
                s.track_ended_ |= s.emu_track_ended_;

                if !s.ignore_silence_ || s.out_time > s.fade_start {
                    // check end for a new run of silence
                    let silence = count_silence(&mut out[pos as usize..]);
                    if silence < remain {
                        s.silence_time = s.emu_time as i64 - silence;
                    }

                    if s.emu_time as i64 - s.silence_time >= BUF_SIZE {
                        fill_buf(self); // cause silence detection on next play()
                    }
                }
            }

            let s = self.emu_mut();
            if s.fade_start >= 0 && s.out_time > s.fade_start {
                handle_fade(s, out);
            }
        }
        let s = self.emu_mut();
        s.out_time = s.out_time.wrapping_add(out_count as i32);
        s.out_time_scaled = s
            .out_time_scaled
            .wrapping_add((out_count as f64 * s.tempo_ / s.out_channels() as f64) as i32);
        Ok(())
    }

    // Informational (Music_Emu)

    /// Sample rate sound is generated at
    fn sample_rate(&self) -> i64 {
        self.emu().sample_rate_
    }

    /// Index of current track or -1 if one hasn't been started
    fn current_track(&self) -> i32 {
        self.emu().current_track_
    }

    /// Number of voices used by currently loaded file
    fn voice_count(&self) -> i32 {
        self.emu().voice_count_
    }

    /// Names of voices
    fn voice_names(&self) -> &[&'static str] {
        &self.emu().voice_names_
    }

    fn multi_channel(&self) -> bool {
        self.emu().multi_channel_
    }

    // Track status/control

    /// Number of milliseconds (1000 msec = 1 second) played since beginning of track
    fn tell(&self) -> i64 {
        let s = self.emu();
        let rate = (s.sample_rate_ as i32).wrapping_mul(s.out_channels());
        let sec = s.out_time / rate;
        sec.wrapping_mul(1000).wrapping_add(
            s.out_time
                .wrapping_sub(sec.wrapping_mul(rate))
                .wrapping_mul(1000)
                / rate,
        ) as i64
    }

    /// Number of samples generated since beginning of track
    fn tell_samples(&self) -> i64 {
        self.emu().out_time as i64
    }

    /// Number of milliseconds played since beginning of track (scaled with tempo).
    fn tell_scaled(&self) -> i64 {
        let s = self.emu();
        (s.out_time_scaled as f64 / (s.sample_rate_ as f64 / 1000.0)) as i64
    }

    /// Seek to new time in track. Seeking backwards or far forward can take a while.
    fn seek(&mut self, msec: i64) -> BlarggErr {
        let n = self.emu().msec_to_samples(msec as i32);
        self.seek_samples(n as i64)
    }

    /// Equivalent to restarting track then skipping n samples
    fn seek_samples(&mut self, time: i64) -> BlarggErr {
        if time < self.emu().out_time as i64 {
            let t = self.emu().current_track_;
            self.start_track(t)?;
        }
        let out_time = self.emu().out_time as i64;
        self.skip(time - out_time)
    }

    /// Seek to new time in track (scaled with tempo).
    fn seek_scaled(&mut self, msec: i64) -> BlarggErr {
        // require( tempo_ > 0 );
        let frames = ((msec as f64 / 1000.0) * self.emu().sample_rate_ as f64) as i32;
        if frames < self.emu().out_time_scaled {
            let t = self.emu().current_track_;
            self.start_track(t)?;
        }
        let s = self.emu();
        let mut samples_to_skip = (frames
            .wrapping_sub(s.out_time_scaled)
            .wrapping_mul(s.out_channels()) as f64
            / s.tempo_) as i32;
        samples_to_skip += samples_to_skip % s.out_channels();
        self.skip(samples_to_skip as i64)
    }

    /// Skip n samples
    fn skip(&mut self, mut count: i64) -> BlarggErr {
        // require( current_track() >= 0 ); // start_track() must have been called already
        {
            let s = self.emu_mut();
            s.out_time = s.out_time.wrapping_add(count as i32);
            s.out_time_scaled = s
                .out_time_scaled
                .wrapping_add((count as f64 * s.tempo_ / s.out_channels() as f64) as i32);

            // remove from silence and buf first
            let n = count.min(s.silence_count);
            s.silence_count -= n;
            count -= n;

            let n = count.min(s.buf_remain);
            s.buf_remain -= n;
            count -= n;
        }

        if count != 0 && !self.emu().emu_track_ended_ {
            let s = self.emu_mut();
            s.emu_time = s.emu_time.wrapping_add(count as i32);
            let r = self.skip_(count);
            end_track_if_error(self.emu_mut(), r);
        }

        let s = self.emu_mut();
        if (s.silence_count | s.buf_remain) == 0 {
            // caught up to emulator, so update track ended
            s.track_ended_ |= s.emu_track_ended_;
        }

        Ok(())
    }

    /// True if a track has reached its end
    fn track_ended(&self) -> bool {
        self.emu().track_ended_
    }

    /// Set start time and length of track fade out. Once fade ends track_ended() returns
    /// true. Fade time can be changed while track is playing.
    fn set_fade(&mut self, start_msec: i64, length_msec: i64) {
        let s = self.emu_mut();
        s.fade_step = 1i64.max(
            s.sample_rate_ * length_msec
                / (FADE_BLOCK_SIZE * FADE_SHIFT as i64 * 1000 / s.out_channels() as i64),
        ) as i32;
        s.fade_start = s.msec_to_samples(start_msec as i32);
    }

    /// Controls whether or not to automatically load and obey track length
    /// metadata for supported emulators.
    fn autoload_playback_limit(&self) -> bool {
        self.emu().emu_autoload_playback_limit_
    }
    fn set_autoload_playback_limit(&mut self, do_autoload_limit: bool) {
        self.emu_mut().emu_autoload_playback_limit_ = do_autoload_limit;
    }

    /// Disable automatic end-of-track detection and skipping of silence at beginning
    fn ignore_silence(&mut self, disable: bool) {
        self.emu_mut().ignore_silence_ = disable;
    }

    // Sound customization

    /// Adjust song tempo, where 1.0 = normal, 0.5 = half speed, 2.0 = double speed.
    /// Track length as returned by track_info() assumes a tempo of 1.0.
    fn set_tempo(&mut self, mut t: f64) {
        // require( sample_rate() ); // sample rate must be set first
        let min = 0.02;
        let max = 4.00;
        if t < min {
            t = min;
        }
        if t > max {
            t = max;
        }
        self.emu_mut().tempo_ = t;
        self.set_tempo_(t);
    }

    /// Mute/unmute voice i, where voice 0 is first voice
    fn mute_voice(&mut self, index: i32, mute: bool) {
        // require( (unsigned) index < (unsigned) voice_count() );
        let bit = 1i32.wrapping_shl(index as u32);
        let mut mask = self.emu().mute_mask_ | bit;
        if !mute {
            mask ^= bit;
        }
        self.mute_voices(mask);
    }

    /// Set muting state of all voices at once using a bit mask, where -1 mutes them all,
    /// 0 unmutes them all, 0x01 mutes just the first voice, etc.
    fn mute_voices(&mut self, mask: i32) {
        // require( sample_rate() ); // sample rate must be set first
        self.emu_mut().mute_mask_ = mask;
        self.mute_voices_(mask);
    }

    /// Disables echo effect at SPC files
    fn disable_echo(&mut self, disable: bool) {
        self.disable_echo_(disable);
    }

    /// Enables/disables accurate emulation options, if any are supported. Might change
    /// equalizer settings.
    fn enable_accuracy(&mut self, enable: bool) {
        self.enable_accuracy_(enable);
    }

    /// Current frequency equalizater parameters
    fn equalizer(&self) -> Equalizer {
        self.emu().equalizer_
    }

    /// Set frequency equalizer parameters
    fn set_equalizer(&mut self, eq: &Equalizer) {
        self.emu_mut().equalizer_ = *eq;
        self.set_equalizer_(eq);
    }

    /// `remute_voices()`
    fn remute_voices(&mut self) {
        let m = self.emu().mute_mask_;
        self.mute_voices(m);
    }
}

/// `Music_Emu::unload()`
pub fn music_emu_unload<E: MusicEmu + ?Sized>(this: &mut E) {
    let s = this.emu_mut();
    s.voice_count_ = 0;
    s.clear_track_vars();
    gme_file::gme_file_unload(this);
}

/// `Music_Emu::pre_load()`
pub fn music_emu_pre_load<E: MusicEmu + ?Sized>(this: &mut E) {
    // require( sample_rate() ); // set_sample_rate() must be called before loading a file
    gme_file_pre_load(this);
}

/// `Gme_File::pre_load()`: public load functions call this at beginning
pub fn gme_file_pre_load<E: MusicEmu + ?Sized>(this: &mut E) {
    this.unload();
}

/// `Music_Emu::post_load_()`
pub fn music_emu_post_load_<E: MusicEmu + ?Sized>(this: &mut E) {
    let t = this.emu().tempo_;
    this.set_tempo(t);
    this.remute_voices();
}

/// `Music_Emu::start_track_()`
pub fn music_emu_start_track_<E: MusicEmu + ?Sized>(this: &mut E, track: i32) -> BlarggErr {
    if this.type_().track_count == 1 {
        let f = &this.emu().file;
        let (pos, len) = (f.track_pos(track), f.track_size(track));
        return this.load_mem_(pos, len);
    }
    Ok(())
}

fn end_track_if_error(s: &mut MusicEmuState, err: BlarggErr) {
    if let Err(e) = err {
        s.emu_track_ended_ = true;
        s.set_warning(e);
    }
}

/// `Music_Emu::skip_()`
pub fn music_emu_skip_<E: MusicEmu + ?Sized>(this: &mut E, mut count: i64) -> BlarggErr {
    let mut buf = std::mem::take(&mut this.emu_mut().buf);
    let r = (|| {
        // for long skip, mute sound
        let threshold: i64 = 30000;
        if count > threshold {
            let saved_mute = this.emu().mute_mask_;
            this.mute_voices(!0);

            while count > threshold / 2 && !this.emu().emu_track_ended_ {
                this.play_(&mut buf[..BUF_SIZE as usize])?;
                count -= BUF_SIZE;
            }

            this.mute_voices(saved_mute);
        }

        while count != 0 && !this.emu().emu_track_ended_ {
            let mut n = BUF_SIZE;
            if n > count {
                n = count;
            }
            count -= n;
            this.play_(&mut buf[..n as usize])?;
        }
        Ok(())
    })();
    this.emu_mut().buf = buf;
    r
}

// Fading

// unit / pow( 2.0, (double) x / step )
fn int_log(x: i32, step: i32, unit: i32) -> i32 {
    let shift = x / step;
    let fraction = x.wrapping_sub(shift.wrapping_mul(step)).wrapping_mul(unit) / step;
    // Note (upstream): a shift past 31 (or a negative one, before the
    // fade starts) is undefined; this shifts by its low five bits, as the
    // x86 instruction upstream's build uses does.
    ((unit - fraction) + (fraction >> 1)).wrapping_shr(shift as u32)
}

fn handle_fade(s: &mut MusicEmuState, out: &mut [Sample]) {
    let out_count = out.len() as i64;
    let mut i: i64 = 0;
    while i < out_count {
        let shift = 14;
        let unit = 1 << shift;
        let gain = int_log(
            (s.out_time.wrapping_add(i as i32).wrapping_sub(s.fade_start) as i64 / FADE_BLOCK_SIZE)
                as i32,
            s.fade_step,
            unit,
        );
        if gain < (unit >> FADE_SHIFT) {
            s.track_ended_ = true;
            s.emu_track_ended_ = true;
        }

        let count = FADE_BLOCK_SIZE.min(out_count - i);
        for io in out[i as usize..(i + count) as usize].iter_mut() {
            *io = ((*io as i32 * gain) >> shift) as Sample;
        }
        i += FADE_BLOCK_SIZE;
    }
}

// Silence detection

fn emu_play<E: MusicEmu + ?Sized>(this: &mut E, out: &mut [Sample]) {
    // check( current_track_ >= 0 );
    let s = this.emu_mut();
    s.emu_time = s.emu_time.wrapping_add(out.len() as i32);
    if s.current_track_ >= 0 && !s.emu_track_ended_ {
        let r = this.play_(out);
        end_track_if_error(this.emu_mut(), r);
    } else {
        out.fill(0);
    }
}

/// number of consecutive silent samples at end
fn count_silence(begin: &mut [Sample]) -> i64 {
    let size = begin.len();
    let first = begin[0];
    begin[0] = SILENCE_THRESHOLD as Sample; // sentinel
    let mut p = size;
    loop {
        p -= 1;
        if (begin[p] as i32 + SILENCE_THRESHOLD / 2) as u32 > SILENCE_THRESHOLD as u32 {
            break;
        }
    }
    begin[0] = first;
    (size - p) as i64
}

/// fill internal buffer and check it for silence
fn fill_buf<E: MusicEmu + ?Sized>(this: &mut E) {
    // assert( !buf_remain );
    if !this.emu().emu_track_ended_ {
        let mut buf = std::mem::take(&mut this.emu_mut().buf);
        emu_play(this, &mut buf[..BUF_SIZE as usize]);
        let silence = count_silence(&mut buf[..BUF_SIZE as usize]);
        let s = this.emu_mut();
        s.buf = buf;
        if silence < BUF_SIZE {
            s.silence_time = s.emu_time as i64 - silence;
            s.buf_remain = BUF_SIZE;
            return;
        }
    }
    this.emu_mut().silence_count += BUF_SIZE;
}

// Gme_Info_: base class for info-only derivations

/// `Gme_Info_::set_sample_rate_()`
pub fn gme_info_set_sample_rate_(_sample_rate: i64) -> BlarggErr {
    Ok(())
}

/// `Gme_Info_::start_track_()` and `Gme_Info_::play_()`
pub const GME_INFO_PLAY_ERR: &str = "Use full emulator for playback";

/// The `Gme_Info_` overrides of the [`MusicEmu`] methods, for the info-only
/// file types: `$t` is the type, whose `state` field holds the
/// `MusicEmuState`.
macro_rules! gme_info_overrides {
    () => {
        fn emu(&self) -> &$crate::music_emu::MusicEmuState {
            &self.state
        }
        fn emu_mut(&mut self) -> &mut $crate::music_emu::MusicEmuState {
            &mut self.state
        }
        fn set_sample_rate_(&mut self, sample_rate: i64) -> $crate::blargg_common::BlarggErr {
            $crate::music_emu::gme_info_set_sample_rate_(sample_rate)
        }
        fn pre_load(&mut self) {
            $crate::music_emu::gme_file_pre_load(self); // skip Music_Emu
        }
        fn post_load_(&mut self) {
            // Gme_File::post_load_(): skip Music_Emu
        }
        fn set_equalizer_(&mut self, _eq: &$crate::music_emu::Equalizer) {
            // check( false );
        }
        fn enable_accuracy_(&mut self, _enable: bool) {
            // check( false );
        }
        fn mute_voices_(&mut self, _mask: i32) {
            // check( false );
        }
        fn set_tempo_(&mut self, _t: f64) {}
        fn start_track_(&mut self, _track: i32) -> $crate::blargg_common::BlarggErr {
            Err($crate::music_emu::GME_INFO_PLAY_ERR)
        }
        fn play_(
            &mut self,
            _out: &mut [$crate::music_emu::Sample],
        ) -> $crate::blargg_common::BlarggErr {
            Err($crate::music_emu::GME_INFO_PLAY_ERR)
        }
    };
}
pub(crate) use gme_info_overrides;
