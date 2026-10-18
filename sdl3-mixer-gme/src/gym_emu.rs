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
// Modified 2026-10-18: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! Sega Genesis/Mega Drive GYM music file emulator
//! Includes with PCM timing recovery to improve sample quality.
//! (`Gym_Emu.h`, `Gym_Emu.cpp`)

use crate::blargg_common::{get_le32, BlarggErr};
use crate::blip_buffer::{BlipBuffer, BlipEq, BlipSynth, BlipTime, BLIP_MED_QUALITY};
use crate::dual_resampler::{DualResampler, DualResamplerHost};
use crate::gme_file::{gme_copy_field, GmeTypeT, TrackInfo, GME_WRONG_FILE_TYPE};
use crate::music_emu::{
    gme_info_overrides, music_emu_start_track_, MusicEmu, MusicEmuState, Sample,
};
use crate::sms_apu::SmsApu;
use crate::ym2612_nuked::Ym2612Emu;

const MIN_TEMPO: f64 = 0.25;
const OVERSAMPLE_FACTOR: f64 = 5.0 / 3.0;
const FM_GAIN: f64 = 3.0;

const BASE_CLOCK: i64 = 53700300;
const CLOCK_RATE: i64 = BASE_CLOCK / 15;

/// GYM file header
pub const HEADER_SIZE: usize = 428;

/// The fields of the GYM `header_t`, as offsets into its bytes.
pub mod header {
    pub const TAG: usize = 0; // [4]
    pub const SONG: usize = 4; // [32]
    pub const GAME: usize = 36; // [32]
    pub const COPYRIGHT: usize = 68; // [32]
    pub const EMULATOR: usize = 100; // [32]
    pub const DUMPER: usize = 132; // [32]
    pub const COMMENT: usize = 164; // [256]
    pub const LOOP_START: usize = 420; // [4] in 1/60 seconds, 0 if not looped
    pub const PACKED: usize = 424; // [4]
}

pub const GYM_RATE: i32 = 60;

/// The id of `blip_buf` in the one-buffer slice its synthesis is given
const BLIP_BUF: usize = 0;

static VOICE_NAMES: [&str; 8] = ["FM 1", "FM 2", "FM 3", "FM 4", "FM 5", "FM 6", "PCM", "PSG"];

/// `Gym_Emu` (its fields are in `imp`, and `dual` is its `Dual_Resampler`
/// base)
#[derive(Debug)]
pub struct GymEmu {
    imp: GymEmuImpl,
    dual: DualResampler,
}

impl Default for GymEmu {
    fn default() -> Self {
        Self::new()
    }
}

/// The fields of `Gym_Emu` and its `Music_Emu`. The sequence's positions
/// (its pointers) are offsets from `data`.
pub struct GymEmuImpl {
    state: MusicEmuState,

    // sequence data begin, loop begin, current position, end
    /// `data`: where the sequence starts in `file_data`
    data: usize,
    loop_begin: Option<u64>,
    pos: u64,
    /// `data_end - data`
    data_size: u64,
    loop_remain: i32, // frames remaining until loop beginning has been located
    header_: [u8; HEADER_SIZE],
    fm_sample_rate: f64,
    clocks_per_frame: i32,

    // dac (pcm)
    dac_amp: i32,
    prev_dac_count: i32,
    dac_enabled: bool,
    dac_muted: bool,

    // sound
    blip_buf: BlipBuffer,
    fm: Ym2612Emu,
    dac_synth: BlipSynth,
    apu: SmsApu,
    dac_buf: [u8; 1024],
}

impl std::fmt::Debug for GymEmuImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GymEmuImpl")
            .field("pos", &self.pos)
            .finish_non_exhaustive()
    }
}

// Track info

/// `strcmp( h.field, s )` is 0 (reading on past the field, as `strcmp()`
/// would, when the field matches without its end)
fn strcmp_eq(h: &[u8], at: usize, s: &[u8]) -> bool {
    for i in 0..=s.len() {
        let c = h.get(at + i).copied().unwrap_or(0);
        if c != s.get(i).copied().unwrap_or(0) {
            return false;
        }
        if c == 0 {
            break;
        }
    }
    true
}

/// `get_gym_info()`, on the header's bytes (only the tag's when it isn't
/// "GYMX")
fn get_gym_info(h: &[u8], mut length: i64, out: &mut TrackInfo) {
    if &h[header::TAG..header::TAG + 4] == b"GYMX" {
        length = length * 50 / 3; // 1000 / 60
        let loop_ = get_le32(&h[header::LOOP_START..]) as i64;
        if loop_ != 0 {
            out.intro_length = loop_ * 50 / 3;
            out.loop_length = length - out.intro_length;
        } else {
            out.length = length;
            out.intro_length = length; // make it clear that track is no longer than length
            out.loop_length = 0;
        }

        // more stupidity where the field should have been left
        if !strcmp_eq(h, header::SONG, b"Unknown Song") {
            gme_copy_field(&mut out.song, &h[header::SONG..header::SONG + 32]);
        }

        if !strcmp_eq(h, header::GAME, b"Unknown Game") {
            gme_copy_field(&mut out.game, &h[header::GAME..header::GAME + 32]);
        }

        if !strcmp_eq(h, header::COPYRIGHT, b"Unknown Publisher") {
            gme_copy_field(
                &mut out.copyright,
                &h[header::COPYRIGHT..header::COPYRIGHT + 32],
            );
        }

        if !strcmp_eq(h, header::DUMPER, b"Unknown Person") {
            gme_copy_field(&mut out.dumper, &h[header::DUMPER..header::DUMPER + 32]);
        }

        if !strcmp_eq(h, header::COMMENT, b"Header added by YMAMP") {
            gme_copy_field(&mut out.comment, &h[header::COMMENT..header::COMMENT + 256]);
        }
    }
}

/// `gym_track_length( p, end )`, on the bytes from `p` to `end`
fn gym_track_length(data: &[u8]) -> i64 {
    let mut time: i64 = 0;
    let mut p = 0usize;
    while p < data.len() {
        let c = data[p];
        p += 1;
        match c {
            0 => time += 1,

            1 | 2 => p += 2,

            3 => p += 1,

            _ => {}
        }
    }
    time
}

/// `check_header( in, size, data_offset )`, on the file's bytes
fn check_header(input: &[u8], data_offset: &mut usize) -> BlarggErr {
    if input.len() < 4 {
        return Err(GME_WRONG_FILE_TYPE);
    }

    if &input[..4] == b"GYMX" {
        if input.len() < HEADER_SIZE + 1 {
            return Err(GME_WRONG_FILE_TYPE);
        }

        if input[header::PACKED..header::PACKED + 4] != [0, 0, 0, 0] {
            return Err("Packed GYM file not supported");
        }

        *data_offset = HEADER_SIZE;
    } else if input[0] > 3 {
        return Err(GME_WRONG_FILE_TYPE);
    }

    Ok(())
}

/// `Gym_File`: the info-only GYM reader (`file_begin` to `file_end` being
/// the file data's `len` bytes from `pos`)
pub(crate) struct GymFile {
    state: MusicEmuState,
    pos: usize,
    len: usize,
    data_offset: usize,
}

impl GymFile {
    fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_GYM_TYPE_);
        GymFile {
            state,
            pos: 0,
            len: 0,
            data_offset: 0,
        }
    }
}

impl std::fmt::Debug for GymFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GymFile").finish_non_exhaustive()
    }
}

impl MusicEmu for GymFile {
    gme_info_overrides!();

    fn load_mem_(&mut self, input: usize, size: usize) -> BlarggErr {
        self.pos = input;
        self.len = size;
        self.data_offset = 0;
        let file = &self.state.file.file_data[input..input + size];
        check_header(file, &mut self.data_offset)
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        let file = &self.state.file.file_data[self.pos..self.pos + self.len];
        let length = gym_track_length(&file[self.data_offset..]);
        get_gym_info(file, length, out);
        Ok(())
    }
}

fn new_gym_emu() -> Box<dyn MusicEmu> {
    Box::new(GymEmu::new())
}
fn new_gym_file() -> Box<dyn MusicEmu> {
    Box::new(GymFile::new())
}

pub(crate) static GME_GYM_TYPE_: GmeTypeT = GmeTypeT {
    system: "Sega Genesis",
    track_count: 1,
    new_emu: new_gym_emu,
    new_info: new_gym_file,
    extension_: "GYM",
    flags_: 0,
};

impl GymEmu {
    pub fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_GYM_TYPE_);
        state.set_voice_names(&VOICE_NAMES);
        state.set_silence_lookahead(1); // tracks should already be trimmed
        GymEmu {
            imp: GymEmuImpl {
                state,
                data: 0,
                loop_begin: None,
                pos: 0,
                data_size: 0,
                loop_remain: 0,
                header_: [0; HEADER_SIZE],
                fm_sample_rate: 0.0,
                clocks_per_frame: 0,
                dac_amp: 0,
                prev_dac_count: 0,
                dac_enabled: false,
                dac_muted: false,
                blip_buf: BlipBuffer::new(),
                fm: Ym2612Emu::new(),
                dac_synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
                apu: SmsApu::new(),
                dac_buf: [0; 1024],
            },
            dual: DualResampler::new(),
        }
    }

    /// Header for currently loaded file
    pub fn header(&self) -> &[u8; HEADER_SIZE] {
        &self.imp.header_
    }

    pub fn track_length(&self) -> i64 {
        gym_track_length(self.imp.data())
    }
}

impl GymEmuImpl {
    /// The sequence (`data` to `data_end`)
    fn data(&self) -> &[u8] {
        &self.state.file.file_data[self.data..self.data + self.data_size as usize]
    }

    /// The byte at `data + o`.
    ///
    /// Note (upstream): the sequence is read until a frame's end (a 0)
    /// without checking where the data ends, past it (a last frame without
    /// its end, and the frames after the end, which go on until the track
    /// ends); zeros are read there, here and in upstream's build for the
    /// tests.
    #[inline]
    fn at(&self, o: u64) -> i32 {
        if o < self.data_size {
            self.state.file.file_data[self.data + o as usize] as i32
        } else {
            0
        }
    }

    fn run_dac(&mut self, dac_count: i32) {
        // Guess beginning and end of sample and adjust rate and buffer position accordingly.

        // count dac samples in next frame
        let mut next_dac_count = 0;
        let mut p = self.pos;
        loop {
            let cmd = self.at(p);
            p += 1;
            if cmd == 0 {
                break;
            }
            let data = self.at(p);
            p += 1;
            if cmd <= 2 {
                p += 1;
            }
            if cmd == 1 && data == 0x2A {
                next_dac_count += 1;
            }
        }

        // detect beginning and end of sample
        let mut rate_count = dac_count;
        let mut start = 0;
        if self.prev_dac_count == 0 && next_dac_count != 0 && dac_count < next_dac_count {
            rate_count = next_dac_count;
            start = next_dac_count - dac_count;
        } else if self.prev_dac_count != 0 && next_dac_count == 0 && dac_count < self.prev_dac_count
        {
            rate_count = self.prev_dac_count;
        }

        // Evenly space samples within buffer section being used
        let period = self.blip_buf.resampled_duration(self.clocks_per_frame) / rate_count as u32;

        let mut time = self
            .blip_buf
            .resampled_time(0)
            .wrapping_add(period.wrapping_mul(start as u32))
            .wrapping_add(period >> 1);

        let mut dac_amp = self.dac_amp;
        if dac_amp < 0 {
            dac_amp = self.dac_buf[0] as i32;
        }

        for i in 0..dac_count as usize {
            let delta = self.dac_buf[i] as i32 - dac_amp;
            dac_amp += delta;
            self.dac_synth
                .offset_resampled(time, delta, &mut self.blip_buf);
            time = time.wrapping_add(period);
        }
        self.dac_amp = dac_amp;
    }

    fn parse_frame(&mut self) {
        let mut dac_count: i32 = 0;
        let mut pos = self.pos;

        if self.loop_remain != 0 {
            self.loop_remain = self.loop_remain.wrapping_sub(1);
            if self.loop_remain == 0 {
                self.loop_begin = Some(pos); // find loop on first time through sequence
            }
        }

        loop {
            let cmd = self.at(pos);
            pos += 1;
            if cmd == 0 {
                break;
            }
            let data = self.at(pos);
            pos += 1;
            if cmd == 1 {
                let data2 = self.at(pos);
                pos += 1;
                if data != 0x2A {
                    if data == 0x2B {
                        self.dac_enabled = (data2 & 0x80) != 0;
                    }

                    self.fm.write0(data, data2);
                } else if dac_count < self.dac_buf.len() as i32 {
                    self.dac_buf[dac_count as usize] = data2 as u8;
                    dac_count += self.dac_enabled as i32;
                }
            } else if cmd == 2 {
                let data2 = self.at(pos);
                pos += 1;
                self.fm.write1(data, data2);
            } else if cmd == 3 {
                self.apu
                    .write_data(std::slice::from_mut(&mut self.blip_buf), 0, data);
            } else {
                // to do: many GYM streams are full of errors, and error count should
                // reflect cases where music is really having problems
                //log_error();
                pos -= 1; // put data back
            }
        }

        // loop
        if pos >= self.data_size {
            // check( pos == data_end );

            if let Some(loop_begin) = self.loop_begin {
                pos = loop_begin;
            } else {
                self.state.set_track_ended();
            }
        }
        self.pos = pos;

        // dac
        if dac_count != 0 && !self.dac_muted {
            self.run_dac(dac_count);
        }
        self.prev_dac_count = dac_count;
    }
}

impl DualResamplerHost for GymEmuImpl {
    fn play_frame(&mut self, blip_time: BlipTime, sample_count: i32, buf: &mut [Sample]) -> i32 {
        if !self.state.track_ended() {
            self.parse_frame();
        }

        self.apu
            .end_frame(std::slice::from_mut(&mut self.blip_buf), blip_time);

        let n = (sample_count.max(0) as usize).min(buf.len());
        buf[..n].fill(0);
        self.fm.run(sample_count >> 1, buf);

        sample_count
    }

    fn blip_buf(&mut self) -> &mut BlipBuffer {
        &mut self.blip_buf
    }
}

impl MusicEmu for GymEmu {
    fn emu(&self) -> &MusicEmuState {
        &self.imp.state
    }
    fn emu_mut(&mut self) -> &mut MusicEmuState {
        &mut self.imp.state
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        get_gym_info(&self.imp.header_, self.track_length(), out);
        Ok(())
    }

    // Setup

    fn set_sample_rate_(&mut self, sample_rate: i64) -> BlarggErr {
        let e = &mut self.imp;
        let eq = BlipEq::with_rolloff(-32.0, 8000, sample_rate, 0);
        e.apu.treble_eq(&eq);
        e.dac_synth.treble_eq(&eq);
        e.apu.volume(0.135 * FM_GAIN * e.state.gain());
        e.dac_synth.volume(0.125 / 256.0 * FM_GAIN * e.state.gain());
        let factor = self
            .dual
            .setup(OVERSAMPLE_FACTOR, 0.990, FM_GAIN * e.state.gain());
        e.fm_sample_rate = sample_rate as f64 * factor;

        e.blip_buf
            .set_sample_rate(sample_rate, (1000.0 / 60.0 / MIN_TEMPO) as i32)?;
        e.blip_buf.set_clock_rate(CLOCK_RATE as u32);

        e.fm.set_rate(e.fm_sample_rate, BASE_CLOCK as f64 / 7.0)?;
        self.dual
            .reset((1.0 / 60.0 / MIN_TEMPO * sample_rate as f64) as i64 as i32)?;

        Ok(())
    }

    fn set_tempo_(&mut self, t: f64) {
        if t < MIN_TEMPO {
            self.set_tempo(MIN_TEMPO);
            return;
        }

        let e = &mut self.imp;
        if e.blip_buf.sample_rate() != 0 {
            e.clocks_per_frame = ((CLOCK_RATE / 60) as f64 / e.state.tempo()) as i64 as i32;
            self.dual
                .resize((e.state.sample_rate() as f64 / (60.0 * e.state.tempo())) as i64 as i32);
        }
    }

    fn mute_voices_(&mut self, mask: i32) {
        // Music_Emu::mute_voices_( mask );
        let e = &mut self.imp;
        e.fm.mute_voices(mask);
        e.dac_muted = (mask & 0x40) != 0;
        let out = if mask & 0x80 != 0 {
            None
        } else {
            Some(BLIP_BUF)
        };
        e.apu.output(out, out, out);
    }

    fn load_mem_(&mut self, input: usize, size: usize) -> BlarggErr {
        // blaarg_static_assert( offsetof (header_t,packed [4]) == header_size, "GYM Header layout incorrect!" );
        let mut offset = 0;
        let e = &mut self.imp;
        check_header(&e.state.file.file_data[input..input + size], &mut offset)?;
        e.state.set_voice_count(8);

        e.data = input + offset;
        e.data_size = (size - offset) as u64;
        e.loop_begin = None;

        if offset != 0 {
            e.header_
                .copy_from_slice(&e.state.file.file_data[input..input + HEADER_SIZE]);
        } else {
            e.header_ = [0; HEADER_SIZE];
        }

        Ok(())
    }

    // Emulation

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        music_emu_start_track_(self, track)?;

        let e = &mut self.imp;
        e.pos = 0;
        e.loop_remain = get_le32(&e.header_[header::LOOP_START..]) as i32;

        e.prev_dac_count = 0;
        e.dac_enabled = false;
        e.dac_amp = -1;

        e.fm.reset();
        e.apu.reset(0, 0);
        e.blip_buf.clear(true);
        self.dual.clear();
        Ok(())
    }

    fn play_(&mut self, out: &mut [Sample]) -> BlarggErr {
        self.dual.dual_play(out.len() as i64, out, &mut self.imp);
        Ok(())
    }
}
