// Blip_Buffer 0.4.1. http://www.slack.net/~ant/

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

//! Multi-channel sound buffer interface, and basic stereo buffer
//! (`Multi_Buffer.h`, `Multi_Buffer.cpp`; `Mono_Buffer` and
//! `Silent_Buffer`, which nothing in the library uses, are left out).

use crate::blargg_common::BlarggErr;
use crate::blip_buffer::{
    BlipBuffer, BlipReader, BlipSample, BlipTime, BufId, BLIP_DEFAULT_LENGTH,
};
use crate::effects_buffer::EffectsBuffer;

/// Get indexed channel, from 0 to channel count - 1
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Channel {
    pub center: Option<BufId>,
    pub left: Option<BufId>,
    pub right: Option<BufId>,
}

pub const TYPE_INDEX_MASK: i32 = 0xFF;
pub const WAVE_TYPE: i32 = 0x100;
pub const NOISE_TYPE: i32 = 0x200;
pub const MIXED_TYPE: i32 = WAVE_TYPE | NOISE_TYPE;

/// The fields of `Multi_Buffer` itself.
#[derive(Debug)]
pub struct MultiBufferBase {
    channels_changed_count_: u32,
    sample_rate_: i64,
    length_: i32,
    samples_per_frame_: i32,
}

impl MultiBufferBase {
    pub fn new(spf: i32) -> Self {
        MultiBufferBase {
            length_: 0,
            sample_rate_: 0,
            channels_changed_count_: 1,
            samples_per_frame_: spf,
        }
    }

    /// `Multi_Buffer::set_sample_rate()`
    pub fn set_sample_rate(&mut self, rate: i64, msec: i32) -> BlarggErr {
        self.sample_rate_ = rate;
        self.length_ = msec;
        Ok(())
    }

    pub fn channels_changed(&mut self) {
        self.channels_changed_count_ = self.channels_changed_count_.wrapping_add(1);
    }
}

/// Interface to one or more Blip_Buffers mapped to one or more channels
/// consisting of left, center, and right buffers.
pub trait MultiBuffer {
    fn base(&self) -> &MultiBufferBase;

    /// The buffers the channels' indices refer to.
    fn bufs(&mut self) -> &mut [BlipBuffer];

    /// Set the number of channels available
    fn set_channel_count(&mut self, _n: i32) -> BlarggErr {
        Ok(())
    }

    fn channel(&self, index: i32, type_: i32) -> Channel;

    // See Blip_Buffer.h
    fn set_sample_rate(&mut self, rate: i64, msec: i32) -> BlarggErr;
    fn clock_rate(&mut self, rate: u32);
    fn bass_freq(&mut self, freq: i32);
    fn clear(&mut self);
    fn end_frame(&mut self, t: BlipTime);
    fn read_samples(&mut self, out: &mut [BlipSample], count: i64) -> i64;
    fn samples_avail(&self) -> i64;

    /// The buffer as an `Effects_Buffer`, if it is one (for
    /// `gme_set_stereo_depth()`, which upstream casts to it).
    fn as_effects_buffer(&mut self) -> Option<&mut EffectsBuffer> {
        None
    }

    fn sample_rate(&self) -> i64 {
        self.base().sample_rate_
    }

    /// Length of buffer, in milliseconds
    fn length(&self) -> i32 {
        self.base().length_
    }

    /// Number of samples per output frame (1 = mono, 2 = stereo)
    fn samples_per_frame(&self) -> i32 {
        self.base().samples_per_frame_
    }

    /// Count of changes to channel configuration. Incremented whenever
    /// a change is made to any of the Blip_Buffers for any channel.
    fn channels_changed_count(&self) -> u32 {
        self.base().channels_changed_count_
    }
}

/// Uses three buffers (one for center) and outputs stereo sample pairs.
#[derive(Debug)]
pub struct StereoBuffer {
    base: MultiBufferBase,
    pub bufs: [BlipBuffer; 3],
    chan: Channel,
    stereo_added: i32,
    was_stereo: i32,
}

impl Default for StereoBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl StereoBuffer {
    pub fn new() -> Self {
        StereoBuffer {
            base: MultiBufferBase::new(2),
            bufs: [BlipBuffer::new(), BlipBuffer::new(), BlipBuffer::new()],
            chan: Channel {
                center: Some(0),
                left: Some(1),
                right: Some(2),
            },
            // (upstream leaves them uninitialized until clear())
            stereo_added: 0,
            was_stereo: 0,
        }
    }

    // Buffers used for all channels
    pub fn center(&mut self) -> &mut BlipBuffer {
        &mut self.bufs[0]
    }
    pub fn left(&mut self) -> &mut BlipBuffer {
        &mut self.bufs[1]
    }
    pub fn right(&mut self) -> &mut BlipBuffer {
        &mut self.bufs[2]
    }

    fn mix_stereo(&mut self, out: &mut [BlipSample], count: i32) {
        let bass = self.bufs[1].bass_shift_;
        let mut left = BlipReader::begin(&self.bufs[1]);
        let mut right = BlipReader::begin(&self.bufs[2]);
        let mut center = BlipReader::begin(&self.bufs[0]);

        for n in 0..count as usize {
            let c = center.read();
            let mut l = c.wrapping_add(left.read());
            let mut r = c.wrapping_add(right.read());
            if l as i16 as i32 != l {
                l = 0x7FFF - (l >> 24);
            }

            center.next(&self.bufs[0], bass);
            if r as i16 as i32 != r {
                r = 0x7FFF - (r >> 24);
            }

            left.next(&self.bufs[1], bass);
            right.next(&self.bufs[2], bass);

            out[n * 2] = l as BlipSample;
            out[n * 2 + 1] = r as BlipSample;
        }

        center.end(&mut self.bufs[0]);
        right.end(&mut self.bufs[2]);
        left.end(&mut self.bufs[1]);
    }

    fn mix_stereo_no_center(&mut self, out: &mut [BlipSample], count: i32) {
        let bass = self.bufs[1].bass_shift_;
        let mut left = BlipReader::begin(&self.bufs[1]);
        let mut right = BlipReader::begin(&self.bufs[2]);

        for n in 0..count as usize {
            let mut l = left.read();
            if l as i16 as i32 != l {
                l = 0x7FFF - (l >> 24);
            }

            let mut r = right.read();
            if r as i16 as i32 != r {
                r = 0x7FFF - (r >> 24);
            }

            left.next(&self.bufs[1], bass);
            right.next(&self.bufs[2], bass);

            out[n * 2] = l as BlipSample;
            out[n * 2 + 1] = r as BlipSample;
        }

        right.end(&mut self.bufs[2]);
        left.end(&mut self.bufs[1]);
    }

    fn mix_mono(&mut self, out: &mut [BlipSample], count: i32) {
        let bass = self.bufs[0].bass_shift_;
        let mut center = BlipReader::begin(&self.bufs[0]);

        for n in 0..count as usize {
            let mut s = center.read();
            if s as i16 as i32 != s {
                s = 0x7FFF - (s >> 24);
            }

            center.next(&self.bufs[0], bass);
            out[n * 2] = s as BlipSample;
            out[n * 2 + 1] = s as BlipSample;
        }

        center.end(&mut self.bufs[0]);
    }
}

impl MultiBuffer for StereoBuffer {
    fn base(&self) -> &MultiBufferBase {
        &self.base
    }

    fn bufs(&mut self) -> &mut [BlipBuffer] {
        &mut self.bufs
    }

    fn channel(&self, _index: i32, _type: i32) -> Channel {
        self.chan
    }

    fn set_sample_rate(&mut self, rate: i64, msec: i32) -> BlarggErr {
        for b in self.bufs.iter_mut() {
            b.set_sample_rate(rate, msec)?;
        }
        let (r, l) = (self.bufs[0].sample_rate(), self.bufs[0].length());
        self.base.set_sample_rate(r, l)
    }

    fn clock_rate(&mut self, rate: u32) {
        for b in self.bufs.iter_mut() {
            b.set_clock_rate(rate);
        }
    }

    fn bass_freq(&mut self, bass: i32) {
        for b in self.bufs.iter_mut() {
            b.bass_freq(bass);
        }
    }

    fn clear(&mut self) {
        self.stereo_added = 0;
        self.was_stereo = 0;
        for b in self.bufs.iter_mut() {
            b.clear(true);
        }
    }

    fn end_frame(&mut self, clock_count: BlipTime) {
        self.stereo_added = 0;
        for (i, b) in self.bufs.iter_mut().enumerate() {
            self.stereo_added |= b.clear_modified() << i;
            b.end_frame(clock_count);
        }
    }

    fn samples_avail(&self) -> i64 {
        self.bufs[0].samples_avail() * 2
    }

    fn read_samples(&mut self, out: &mut [BlipSample], count: i64) -> i64 {
        // require( !(count & 1) ); // count must be even
        let mut count = (count as u32 / 2) as i64;

        let avail = self.bufs[0].samples_avail();
        if count > avail {
            count = avail;
        }
        if count != 0 {
            let bufs_used = self.stereo_added | self.was_stereo;
            //debug_printf( "%X\n", bufs_used );
            if bufs_used <= 1 {
                self.mix_mono(out, count as i32);
                self.bufs[0].remove_samples(count);
                self.bufs[1].remove_silence(count);
                self.bufs[2].remove_silence(count);
            } else if bufs_used & 1 != 0 {
                self.mix_stereo(out, count as i32);
                self.bufs[0].remove_samples(count);
                self.bufs[1].remove_samples(count);
                self.bufs[2].remove_samples(count);
            } else {
                self.mix_stereo_no_center(out, count as i32);
                self.bufs[0].remove_silence(count);
                self.bufs[1].remove_samples(count);
                self.bufs[2].remove_samples(count);
            }

            // to do: this might miss opportunities for optimization
            if self.bufs[0].samples_avail() == 0 {
                self.was_stereo = self.stereo_added;
                self.stereo_added = 0;
            }
        }

        count * 2
    }
}

/// `blip_default_length`, as `set_sample_rate()`'s default.
pub const DEFAULT_LENGTH: i32 = BLIP_DEFAULT_LENGTH;
