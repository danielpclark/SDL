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

//! Low-level parts of Vgm_Emu (`Vgm_Emu_Impl.h`, `Vgm_Emu_Impl.cpp`).
//!
//! [`VgmEmuImpl`] holds `Vgm_Emu_Impl`'s fields with its `Classic_Emu`'s
//! and `Vgm_Emu`'s; the [`VgmEmu`](crate::vgm_emu::VgmEmu) holds it and
//! the `Dual_Resampler` (`Vgm_Emu_Impl`'s private base), which plays the
//! frames this makes.

use std::ops::{Deref, DerefMut};

use crate::blip_buffer::{BlipBuffer, BlipSynth, BlipTime, BLIP_MED_QUALITY};
use crate::classic_emu::ClassicEmuState;
use crate::dual_resampler::DualResamplerHost;
use crate::music_emu::{MusicEmuState, Sample};
use crate::sms_apu::SmsApu;
use crate::ym2413_emu::Ym2413Emu;
use crate::ym2612_nuked::Ym2612Emu;

const CMD_GG_STEREO: i32 = 0x4F;
const CMD_PSG: i32 = 0x50;
const CMD_YM2413: i32 = 0x51;
const CMD_YM2612_PORT0: i32 = 0x52;
const CMD_YM2612_PORT1: i32 = 0x53;
const CMD_YM2151: i32 = 0x54;
const CMD_DELAY: i32 = 0x61;
const CMD_DELAY_735: i32 = 0x62;
const CMD_DELAY_882: i32 = 0x63;
const CMD_BYTE_DELAY: i32 = 0x64;
const CMD_END: i32 = 0x66;
const CMD_DATA_BLOCK: i32 = 0x67;
const CMD_SHORT_DELAY: i32 = 0x70;
const CMD_PCM_DELAY: i32 = 0x80;
const CMD_PCM_SEEK: i32 = 0xE0;

const CMD_GG_STEREO_2: i32 = 0x3F;
const CMD_PSG_2: i32 = 0x30;
const CMD_YM2413_2: i32 = 0xA1;
const CMD_YM2612_2_PORT0: i32 = 0xA2;
const CMD_YM2612_2_PORT1: i32 = 0xA3;

const PCM_BLOCK_TYPE: i32 = 0x00;
const YM2612_DAC_PORT: i32 = 0x2A;

pub(crate) const STEREO: i32 = 2;

pub(crate) const FM_TIME_BITS: u32 = 12;
pub(crate) const BLIP_TIME_BITS: u32 = 12;

/// The id of `blip_buf` in the one-buffer slice its synthesis is given
/// (the PSG's in FM mode, and the DAC's).
pub(crate) const BLIP_BUF: usize = 0;

#[inline]
fn command_len(command: i32) -> i32 {
    match command >> 4 {
        0x03 | 0x04 => return 2,

        0x05 | 0x0A | 0x0B => return 3,

        0x0C | 0x0D => return 4,

        0x0E | 0x0F => return 5,

        _ => {}
    }

    // check( false );
    1
}

/// The `Emu` of a [`YmEmu`]: running it into a frame's samples.
pub trait YmChip {
    const OUT_CHAN_COUNT: i32;
    fn run(&mut self, pair_count: i32, out: &mut [i16]);
}

impl YmChip for Ym2612Emu {
    const OUT_CHAN_COUNT: i32 = Ym2612Emu::OUT_CHAN_COUNT;
    fn run(&mut self, pair_count: i32, out: &mut [i16]) {
        Ym2612Emu::run(self, pair_count, out);
    }
}

impl YmChip for Ym2413Emu {
    const OUT_CHAN_COUNT: i32 = Ym2413Emu::OUT_CHAN_COUNT;
    fn run(&mut self, pair_count: i32, out: &mut [i16]) {
        Ym2413Emu::run(self, pair_count, out);
    }
}

const DISABLED_TIME: i32 = -1;

/// `Ym_Emu<Emu>` (which derives from its `Emu`, as this derefs to it).
/// `out` is where the next samples go in the frame's samples, which
/// `run_until()` is given.
#[derive(Debug, Default)]
pub struct YmEmu<E> {
    emu: E,
    last_time: i32,
    out: usize,
}

impl<E> Deref for YmEmu<E> {
    type Target = E;
    fn deref(&self) -> &E {
        &self.emu
    }
}

impl<E> DerefMut for YmEmu<E> {
    fn deref_mut(&mut self) -> &mut E {
        &mut self.emu
    }
}

impl<E: YmChip> YmEmu<E> {
    pub fn new(emu: E) -> Self {
        YmEmu {
            emu,
            last_time: DISABLED_TIME,
            out: 0,
        }
    }
    pub fn enable(&mut self, b: bool) {
        self.last_time = if b { 0 } else { DISABLED_TIME };
    }
    pub fn enabled(&self) -> bool {
        self.last_time != DISABLED_TIME
    }

    #[inline]
    pub fn begin_frame(&mut self) {
        // require( enabled() );
        self.out = 0;
        self.last_time = 0;
    }

    #[inline]
    pub fn run_until(&mut self, time: i32, buf: &mut [i16]) -> bool {
        let count = time.wrapping_sub(self.last_time);
        if count > 0 {
            if self.last_time < 0 {
                return false;
            }
            self.last_time = time;
            let p = self.out;
            self.out = p.wrapping_add((count * E::OUT_CHAN_COUNT) as usize);
            self.emu.run(count, buf.get_mut(p..).unwrap_or(&mut []));
        }
        true
    }
}

/// The fields of `Vgm_Emu_Impl`, its `Classic_Emu` and `Vgm_Emu`. The
/// file positions (`data`'s pointers) are offsets from `data`.
pub struct VgmEmuImpl {
    pub(crate) state: MusicEmuState,
    pub(crate) classic: ClassicEmuState,

    pub(crate) fm_time_offset: i64,
    pub(crate) fm_time_factor: u64,

    pub(crate) blip_time_factor: u64,

    /// `data`: where the file starts in `file_data`
    pub(crate) data: usize,
    pub(crate) loop_begin: u64,
    /// `data_end - data`
    pub(crate) data_size: u64,

    pub(crate) vgm_time: i32,
    pub(crate) pos: u64,

    pub(crate) pcm_data: u64,
    pub(crate) pcm_pos: u64,
    pub(crate) dac_amp: i32,
    pub(crate) dac_disabled: i32, // -1 if disabled

    pub(crate) ym2612: [YmEmu<Ym2612Emu>; 2],
    pub(crate) ym2413: [YmEmu<Ym2413Emu>; 2],

    pub(crate) blip_buf: BlipBuffer,
    pub(crate) psg: [SmsApu; 2],
    pub(crate) psg_dual: bool,
    pub(crate) psg_t6w28: bool,
    pub(crate) dac_synth: BlipSynth,

    // Vgm_Emu
    pub(crate) fm_rate: f64,
    pub(crate) psg_rate: u32,
    pub(crate) vgm_rate: u32,
    pub(crate) disable_oversampling_: bool,
    pub(crate) uses_fm: bool,
}

impl std::fmt::Debug for VgmEmuImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VgmEmuImpl")
            .field("uses_fm", &self.uses_fm)
            .finish_non_exhaustive()
    }
}

/// The buffers the PSGs synthesize into: `blip_buf` with the FM chips,
/// the `Classic_Emu`'s otherwise.
macro_rules! psg_bufs {
    ($e:expr) => {
        if $e.uses_fm {
            std::slice::from_mut(&mut $e.blip_buf)
        } else {
            $e.classic.bufs()
        }
    };
}
pub(crate) use psg_bufs;

impl VgmEmuImpl {
    pub(crate) fn new(state: MusicEmuState, classic: ClassicEmuState) -> Self {
        VgmEmuImpl {
            state,
            classic,
            // FIXME (upstream): left uninitialized until a track with the
            // FM chips starts, and read without them, for their commands;
            // zeroed here, and in upstream's build for the tests.
            fm_time_offset: 0,
            fm_time_factor: 0,
            blip_time_factor: 0,
            data: 0,
            loop_begin: 0,
            data_size: 0,
            vgm_time: 0,
            pos: 0,
            pcm_data: 0,
            pcm_pos: 0,
            dac_amp: 0,
            dac_disabled: 0,
            ym2612: [YmEmu::new(Ym2612Emu::new()), YmEmu::new(Ym2612Emu::new())],
            ym2413: [YmEmu::new(Ym2413Emu::new()), YmEmu::new(Ym2413Emu::new())],
            blip_buf: BlipBuffer::new(),
            psg: [SmsApu::new(), SmsApu::new()],
            psg_dual: false,
            psg_t6w28: false,
            dac_synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
            fm_rate: 0.0,
            psg_rate: 0,
            vgm_rate: 0,
            disable_oversampling_: false,
            uses_fm: false,
        }
    }

    /// The header and data (`data` to `data_end`)
    pub(crate) fn data(&self) -> &[u8] {
        &self.state.file.file_data[self.data..self.data + self.data_size as usize]
    }

    /// The byte at `data + o`.
    ///
    /// FIXME (upstream): the commands are read without checking where the
    /// data ends (a command cut short by the end, a data block, a PCM seek
    /// or the loop past it, the data offset), past it; zeros are read
    /// there, here and in upstream's build for the tests.
    #[inline]
    pub(crate) fn at(&self, o: u64) -> i32 {
        if o < self.data_size {
            self.state.file.file_data[self.data + o as usize] as i32
        } else {
            0
        }
    }

    /// `get_le32( data + o )`, as [`at()`](Self::at) reads
    fn le32(&self, o: u64) -> u32 {
        (self.at(o + 3) as u32) << 24
            | (self.at(o + 2) as u32) << 16
            | (self.at(o + 1) as u32) << 8
            | self.at(o) as u32
    }

    #[inline]
    pub(crate) fn to_fm_time(&self, t: i32) -> i32 {
        ((t as i64 as u64)
            .wrapping_mul(self.fm_time_factor)
            .wrapping_add(self.fm_time_offset as u64)
            >> FM_TIME_BITS) as i32
    }

    #[inline]
    pub(crate) fn to_blip_time(&self, t: i32) -> BlipTime {
        ((t as i64 as u64).wrapping_mul(self.blip_time_factor) >> BLIP_TIME_BITS) as BlipTime
    }

    fn write_pcm(&mut self, vgm_time: i32, amp: i32) {
        let blip_time = self.to_blip_time(vgm_time);
        let old = self.dac_amp;
        let delta = amp - old;
        self.dac_amp = amp;
        if old >= 0 {
            self.dac_synth
                .offset_inline(blip_time, delta, &mut self.blip_buf);
        } else {
            self.dac_amp |= self.dac_disabled;
        }
    }

    /// `run_commands()`, with the FM chips running into `fm_buf` (the
    /// frame's samples; empty without the FM chips)
    pub(crate) fn run_commands(&mut self, end_time: i32, fm_buf: &mut [i16]) -> BlipTime {
        let mut vgm_time = self.vgm_time;
        let mut pos = self.pos;
        let data_end = self.data_size;
        if pos >= data_end {
            self.state.set_track_ended();
            if pos > data_end {
                self.state.set_warning("Stream lacked end event");
            }
        }

        // FIXME (upstream): a loop that takes no time (back to its end
        // command, or through no delay) never ends; the stream ends at its
        // end command when no time has passed since it last looped there,
        // here and in upstream's build for the tests.
        let mut loop_time: Option<i32> = None;
        let mut unknown = false;

        while vgm_time < end_time && pos < data_end {
            // TODO: be sure there are enough bytes left in stream for particular command
            // so we don't read past end
            let cmd = self.at(pos);
            pos += 1;
            match cmd {
                CMD_END => {
                    if loop_time == Some(vgm_time) {
                        pos = data_end;
                    } else {
                        loop_time = Some(vgm_time);
                        pos = self.loop_begin; // if not looped, loop_begin == data_end
                    }
                }

                CMD_DELAY_735 => vgm_time = vgm_time.wrapping_add(735),

                CMD_DELAY_882 => vgm_time = vgm_time.wrapping_add(882),

                CMD_GG_STEREO => {
                    let (t, d) = (self.to_blip_time(vgm_time), self.at(pos));
                    pos += 1;
                    self.psg[0].write_ggstereo(psg_bufs!(self), t, d);
                }

                CMD_PSG => {
                    let (t, d) = (self.to_blip_time(vgm_time), self.at(pos));
                    pos += 1;
                    self.psg[0].write_data(psg_bufs!(self), t, d);
                }

                CMD_GG_STEREO_2 => {
                    let (t, d) = (self.to_blip_time(vgm_time), self.at(pos));
                    pos += 1;
                    self.psg[1].write_ggstereo(psg_bufs!(self), t, d);
                }

                CMD_PSG_2 => {
                    let (t, d) = (self.to_blip_time(vgm_time), self.at(pos));
                    pos += 1;
                    self.psg[1].write_data(psg_bufs!(self), t, d);
                }

                CMD_DELAY => {
                    vgm_time = vgm_time.wrapping_add(self.at(pos + 1) * 0x100 + self.at(pos));
                    pos += 2;
                }

                CMD_BYTE_DELAY => {
                    vgm_time = vgm_time.wrapping_add(self.at(pos));
                    pos += 1;
                }

                CMD_YM2413 | CMD_YM2413_2 => {
                    let i = (cmd == CMD_YM2413_2) as usize;
                    let t = self.to_fm_time(vgm_time);
                    if self.ym2413[i].run_until(t, fm_buf) {
                        let (a, d) = (self.at(pos), self.at(pos + 1));
                        self.ym2413[i].write(a, d);
                    }
                    pos += 2;
                }

                CMD_YM2612_PORT0 | CMD_YM2612_2_PORT0 => {
                    let i = (cmd == CMD_YM2612_2_PORT0) as usize;
                    let (a, d) = (self.at(pos), self.at(pos + 1));
                    if a == YM2612_DAC_PORT {
                        self.write_pcm(vgm_time, d);
                    } else {
                        let t = self.to_fm_time(vgm_time);
                        if self.ym2612[i].run_until(t, fm_buf) {
                            if a == 0x2B {
                                self.dac_disabled = (d >> 7 & 1) - 1;
                                self.dac_amp |= self.dac_disabled;
                            }
                            self.ym2612[i].write0(a, d);
                        }
                    }
                    pos += 2;
                }

                CMD_YM2612_PORT1 | CMD_YM2612_2_PORT1 => {
                    let i = (cmd == CMD_YM2612_2_PORT1) as usize;
                    let t = self.to_fm_time(vgm_time);
                    if self.ym2612[i].run_until(t, fm_buf) {
                        let (a, d) = (self.at(pos), self.at(pos + 1));
                        self.ym2612[i].write1(a, d);
                    }
                    pos += 2;
                }

                CMD_DATA_BLOCK => {
                    // check( *pos == cmd_end );
                    let type_ = self.at(pos + 1);
                    let size = self.le32(pos + 2) as u64;
                    pos += 6;
                    if type_ == PCM_BLOCK_TYPE {
                        self.pcm_data = pos;
                    }
                    pos += size;
                }

                CMD_PCM_SEEK => {
                    self.pcm_pos = self.pcm_data
                        + (self.at(pos + 3) as u64 * 0x1000000
                            + self.at(pos + 2) as u64 * 0x10000
                            + self.at(pos + 1) as u64 * 0x100
                            + self.at(pos) as u64);
                    pos += 4;
                }

                _ => {
                    let cmd = self.at(pos - 1);
                    match cmd & 0xF0 {
                        CMD_PCM_DELAY => {
                            let amp = self.at(self.pcm_pos);
                            self.pcm_pos += 1;
                            self.write_pcm(vgm_time, amp);
                            vgm_time = vgm_time.wrapping_add(cmd & 0x0F);
                        }

                        CMD_SHORT_DELAY => vgm_time = vgm_time.wrapping_add((cmd & 0x0F) + 1),

                        0x50 => pos += 2,

                        _ => {
                            pos = pos.wrapping_add((command_len(cmd) - 1) as u64);
                            unknown = true;
                        }
                    }
                }
            }
        }
        if unknown {
            self.state.set_warning("Unknown stream event");
        }
        vgm_time = vgm_time.wrapping_sub(end_time);
        self.pos = pos;
        self.vgm_time = vgm_time;

        self.to_blip_time(end_time)
    }

    /// Update pre-1.10 header FM rates by scanning commands
    pub(crate) fn update_fm_rates(&self, ym2413_rate: &mut i64, ym2612_rate: &mut i64) {
        let mut p: u64 = 0x40;
        while p < self.data_size {
            match self.at(p) {
                CMD_END => return,

                CMD_PSG | CMD_BYTE_DELAY => p += 2,

                CMD_DELAY => p += 3,

                // FIXME (upstream): the block's size (read past the end
                // of the data, as at() does) is added in 32 bits, which
                // wrap to no advance at all (and the scan never ends) at
                // 0xFFFFFFF9; it is added in 64 bits here, and in
                // upstream's build for the tests.
                CMD_DATA_BLOCK => p += 7 + self.le32(p + 3) as u64,

                CMD_YM2413 => {
                    *ym2612_rate = 0;
                    return;
                }

                CMD_YM2612_PORT0 | CMD_YM2612_PORT1 => {
                    *ym2612_rate = *ym2413_rate;
                    *ym2413_rate = 0;
                    return;
                }

                CMD_YM2151 => {
                    *ym2413_rate = 0;
                    *ym2612_rate = 0;
                    return;
                }

                c => p += command_len(c) as u64,
            }
        }
    }
}

impl DualResamplerHost for VgmEmuImpl {
    fn play_frame(&mut self, blip_time: BlipTime, sample_count: i32, buf: &mut [Sample]) -> i32 {
        // to do: timing is working mostly by luck

        let min_pairs = sample_count >> 1;
        let mut vgm_time: i32 = ((((min_pairs as i64) << FM_TIME_BITS) as u64)
            / self.fm_time_factor)
            .wrapping_sub(1) as i32;
        // assert( to_fm_time( vgm_time ) <= min_pairs );
        let mut pairs;
        loop {
            pairs = self.to_fm_time(vgm_time);
            if pairs >= min_pairs {
                break;
            }
            vgm_time = vgm_time.wrapping_add(1);
        }
        //debug_printf( "pairs: %d, min_pairs: %d\n", pairs, min_pairs );

        let n = ((pairs as i64 * STEREO as i64).max(0) as usize).min(buf.len());
        if self.ym2612[0].enabled() {
            self.ym2612[0].begin_frame();
            if self.ym2612[1].enabled() {
                self.ym2612[1].begin_frame();
            }
            buf[..n].fill(0);
        } else if self.ym2413[0].enabled() {
            self.ym2413[0].begin_frame();
            if self.ym2413[1].enabled() {
                self.ym2413[1].begin_frame();
            }
            buf[..n].fill(0);
        }

        self.run_commands(vgm_time, buf);

        if self.ym2612[0].enabled() {
            self.ym2612[0].run_until(pairs, buf);
        }
        if self.ym2612[1].enabled() {
            self.ym2612[1].run_until(pairs, buf);
        }

        if self.ym2413[0].enabled() {
            self.ym2413[0].run_until(pairs, buf);
        }
        if self.ym2413[1].enabled() {
            self.ym2413[1].run_until(pairs, buf);
        }

        self.fm_time_offset = (vgm_time as i64 as u64)
            .wrapping_mul(self.fm_time_factor)
            .wrapping_add(self.fm_time_offset as u64)
            .wrapping_sub(((pairs as i64) << FM_TIME_BITS) as u64)
            as i64;

        let b = std::slice::from_mut(&mut self.blip_buf);
        self.psg[0].end_frame(b, blip_time);
        if self.psg_dual {
            self.psg[1].end_frame(b, blip_time);
        }

        pairs.wrapping_mul(STEREO)
    }

    fn blip_buf(&mut self) -> &mut BlipBuffer {
        &mut self.blip_buf
    }
}
