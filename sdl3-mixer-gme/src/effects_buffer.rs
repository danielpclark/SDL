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

//! Multi-channel effects buffer with panning, echo and reverb
//! (`Effects_Buffer.h`, `Effects_Buffer.cpp`).

use crate::blargg_common::BlarggErr;
use crate::blip_buffer::{BlipBuffer, BlipReader, BlipSample, BlipTime};
use crate::multi_buffer::{Channel, MultiBuffer, MultiBufferBase, NOISE_TYPE, TYPE_INDEX_MASK};

/// (the class's own `fixed_t`, a `long`, which its members use; the
/// file's `int32_t` one is hidden by it)
type FixedT = i64;

#[inline]
fn to_fixed(f: f64) -> FixedT {
    (f * (1i64 << 15) as f64 + 0.5) as FixedT
}

#[inline]
fn fmul(x: i64, y: i64) -> i64 {
    x.wrapping_mul(y) >> 15
}

const ECHO_SIZE: u32 = 4096;
const ECHO_MASK: u32 = ECHO_SIZE - 1;

const REVERB_SIZE: u32 = 8192 * 2;
const REVERB_MASK: u32 = REVERB_SIZE - 1;

/// Channel configuration
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// -1.0 = left, 0.0 = center, 1.0 = right
    pub pan_1: f64,
    pub pan_2: f64,
    /// msec
    pub echo_delay: f64,
    /// 0.0 to 1.0
    pub echo_level: f64,
    /// msec
    pub reverb_delay: f64,
    /// difference between left/right delays (msec)
    pub delay_variance: f64,
    /// 0.0 to 1.0
    pub reverb_level: f64,
    /// if false, use optimized simple mixer
    pub effects_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            pan_1: -0.15f32 as f64,
            pan_2: 0.15f32 as f64,
            reverb_delay: 88.0f32 as f64,
            reverb_level: 0.12f32 as f64,
            echo_delay: 61.0f32 as f64,
            echo_level: 0.10f32 as f64,
            delay_variance: 18.0f32 as f64,
            effects_enabled: false,
        }
    }
}

#[derive(Debug, Default)]
struct Chans {
    pan_1_levels: [FixedT; 2],
    pan_2_levels: [FixedT; 2],
    echo_delay_l: i32,
    echo_delay_r: i32,
    echo_level: FixedT,
    reverb_delay_l: i32,
    reverb_delay_r: i32,
    reverb_level: FixedT,
}

const MAX_BUF_COUNT: i32 = 7;
const CHAN_TYPES_COUNT: i32 = 3;

/// Effects_Buffer uses several buffers and outputs stereo sample pairs.
#[derive(Debug)]
pub struct EffectsBuffer {
    base: MultiBufferBase,
    max_voices: i32,
    bufs: Vec<BlipBuffer>,
    chan_types: Vec<Channel>,
    config_: Config,
    stereo_remain: i64,
    effect_remain: i64,
    buf_count: i32,
    effects_enabled: bool,

    reverb_buf: Vec<Vec<BlipSample>>,
    echo_buf: Vec<Vec<BlipSample>>,
    reverb_pos: Vec<i32>,
    echo_pos: Vec<i32>,

    chans: Chans,
}

#[inline]
fn pin_range(n: i32, max: i32, min: i32) -> i32 {
    if n < min {
        return min;
    }
    if n > max {
        return max;
    }
    n
}

impl EffectsBuffer {
    /// nVoices indicates the number of voices for which buffers will be allocated
    /// to make Effects_Buffer work as "mix everything to one", nVoices will be 1
    /// If center_only is true, only center buffers are created and
    /// less memory is used.
    pub fn new(num_voices: i32, center_only: bool) -> Self {
        let max_voices = num_voices;
        let buf_count = max_voices
            * if center_only {
                MAX_BUF_COUNT - 4
            } else {
                MAX_BUF_COUNT
            };
        let mut b = EffectsBuffer {
            base: MultiBufferBase::new(2 * num_voices),
            max_voices,
            bufs: (0..buf_count).map(|_| BlipBuffer::new()).collect(),
            chan_types: vec![Channel::default(); (max_voices * CHAN_TYPES_COUNT) as usize],
            config_: Config::default(),
            stereo_remain: 0,
            effect_remain: 0,
            // TODO: Reorder buf_count to be initialized before bufs to factor out channel sizing
            buf_count,
            effects_enabled: false,
            reverb_buf: vec![vec![0; REVERB_SIZE as usize]; max_voices as usize],
            echo_buf: vec![vec![0; ECHO_SIZE as usize]; max_voices as usize],
            reverb_pos: vec![0; max_voices as usize],
            echo_pos: vec![0; max_voices as usize],
            chans: Chans::default(),
        };
        b.set_depth(0.0);
        b
    }

    pub fn set_depth(&mut self, d: f64) {
        let mut f = d as f32;
        let mut c = Config {
            pan_1: (-0.6f32 * f) as f64,
            pan_2: (0.6f32 * f) as f64,
            reverb_delay: (880.0f32 * 0.1f32) as f64,
            echo_delay: (610.0f32 * 0.1f32) as f64,
            ..Config::default()
        };
        if f > 0.5 {
            f = 0.5; // TODO: more linear reduction of extreme reverb/echo
        }
        c.reverb_level = (0.5f32 * f) as f64;
        c.echo_level = (0.30f32 * f) as f64;
        c.delay_variance = (180.0f32 * 0.1f32) as f64;
        c.effects_enabled = d > 0.0f32 as f64;
        self.config(&c);
    }

    /// Set configuration of buffer
    pub fn config(&mut self, cfg: &Config) {
        self.base.channels_changed();

        // clear echo and reverb buffers
        // ensure the echo/reverb buffers have already been allocated, so this method can be
        // called before set_sample_rate is called
        if !self.config_.effects_enabled && cfg.effects_enabled && !self.echo_buf[0].is_empty() {
            for i in 0..self.max_voices as usize {
                self.echo_buf[i].fill(0);
                self.reverb_buf[i].fill(0);
            }
        }

        self.config_ = *cfg;

        let mbc = MAX_BUF_COUNT as usize;
        let ctc = CHAN_TYPES_COUNT as usize;
        if self.config_.effects_enabled {
            // convert to internal format

            self.chans.pan_1_levels[0] = to_fixed(1.0) - to_fixed(self.config_.pan_1);
            self.chans.pan_1_levels[1] = to_fixed(2.0) - self.chans.pan_1_levels[0];

            self.chans.pan_2_levels[0] = to_fixed(1.0) - to_fixed(self.config_.pan_2);
            self.chans.pan_2_levels[1] = to_fixed(2.0) - self.chans.pan_2_levels[0];

            self.chans.reverb_level = to_fixed(self.config_.reverb_level);
            self.chans.echo_level = to_fixed(self.config_.echo_level);

            let sample_rate = self.sample_rate();
            let delay_offset =
                (1.0 / 2000.0 * self.config_.delay_variance * sample_rate as f64) as i32;

            let reverb_sample_delay =
                (1.0 / 1000.0 * self.config_.reverb_delay * sample_rate as f64) as i32;
            self.chans.reverb_delay_l = pin_range(
                (REVERB_SIZE as i32)
                    .wrapping_sub((reverb_sample_delay - delay_offset).wrapping_mul(2)),
                REVERB_SIZE as i32 - 2,
                0,
            );
            self.chans.reverb_delay_r = pin_range(
                (REVERB_SIZE as i32 + 1)
                    .wrapping_sub((reverb_sample_delay + delay_offset).wrapping_mul(2)),
                REVERB_SIZE as i32 - 1,
                1,
            );

            let echo_sample_delay =
                (1.0 / 1000.0 * self.config_.echo_delay * sample_rate as f64) as i32;
            self.chans.echo_delay_l = pin_range(
                (ECHO_SIZE as i32 - 1).wrapping_sub(echo_sample_delay - delay_offset),
                ECHO_SIZE as i32 - 1,
                0,
            );
            self.chans.echo_delay_r = pin_range(
                (ECHO_SIZE as i32 - 1).wrapping_sub(echo_sample_delay + delay_offset),
                ECHO_SIZE as i32 - 1,
                0,
            );

            for i in 0..self.max_voices as usize {
                self.chan_types[i * ctc] = Channel {
                    center: Some(i * mbc),
                    left: Some(i * mbc + 3),
                    right: Some(i * mbc + 4),
                };
                self.chan_types[i * ctc + 1] = Channel {
                    center: Some(i * mbc + 1),
                    left: Some(i * mbc + 3),
                    right: Some(i * mbc + 4),
                };
                self.chan_types[i * ctc + 2] = Channel {
                    center: Some(i * mbc + 2),
                    left: Some(i * mbc + 5),
                    right: Some(i * mbc + 6),
                };
            }
        } else {
            for i in 0..self.max_voices as usize {
                // set up outputs
                for j in 0..ctc {
                    self.chan_types[i * ctc + j] = Channel {
                        center: Some(i * mbc),
                        left: Some(i * mbc + 1),
                        right: Some(i * mbc + 2),
                    };
                }
            }
        }

        if self.buf_count < MAX_BUF_COUNT {
            // if center_only
            for c in self.chan_types.iter_mut() {
                c.left = c.center;
                c.right = c.center;
            }
        }
    }

    fn mix_mono(&mut self, out_: &mut [BlipSample], count: i32) {
        let mv = self.max_voices as usize;
        let mbc = MAX_BUF_COUNT as usize;
        for i in 0..mv {
            let mut out = 0usize;
            let b = &self.bufs[i * mbc];
            let bass = b.bass_shift_;
            let mut c = BlipReader::begin(b);

            // unrolled loop
            for _ in 0..(count >> 1) {
                let mut cs0 = c.read();
                c.next(b, bass);

                let mut cs1 = c.read();
                c.next(b, bass);

                if cs0 as i16 as i32 != cs0 {
                    cs0 = 0x7FFF - (cs0 >> 24);
                }
                out_[out + i * 2] = cs0 as i16;
                out_[out + i * 2 + 1] = cs0 as i16;

                if cs1 as i16 as i32 != cs1 {
                    cs1 = 0x7FFF - (cs1 >> 24);
                }
                out_[out + (i + mv) * 2] = cs1 as i16;
                out_[out + (i + mv) * 2 + 1] = cs1 as i16;
                out += mv * 4;
            }

            if count & 1 != 0 {
                let mut s = c.read();
                c.next(b, bass);
                if s as i16 as i32 != s {
                    s = 0x7FFF - (s >> 24);
                }
                out_[out + i * 2] = s as i16;
                out_[out + i * 2 + 1] = s as i16;
            }

            c.end(&mut self.bufs[i * mbc]);
        }
    }

    fn mix_stereo(&mut self, out_: &mut [BlipSample], frames: i32) {
        let mv = self.max_voices as usize;
        let mbc = MAX_BUF_COUNT as usize;
        for i in 0..mv {
            let mut out = 0usize;
            let bass = self.bufs[i * mbc].bass_shift_;
            let (bc, bl, br) = (
                &self.bufs[i * mbc],
                &self.bufs[i * mbc + 1],
                &self.bufs[i * mbc + 2],
            );
            let mut c = BlipReader::begin(bc);
            let mut l = BlipReader::begin(bl);
            let mut r = BlipReader::begin(br);

            for _ in 0..frames {
                let cs = c.read();
                c.next(bc, bass);
                let mut left = cs.wrapping_add(l.read());
                let mut right = cs.wrapping_add(r.read());
                l.next(bl, bass);
                r.next(br, bass);

                if left as i16 as i32 != left {
                    left = 0x7FFF - (left >> 24);
                }

                if right as i16 as i32 != right {
                    right = 0x7FFF - (right >> 24);
                }

                out_[out + i * 2] = left as i16;
                out_[out + i * 2 + 1] = right as i16;

                out += mv * 2;
            }

            r.end(&mut self.bufs[i * mbc + 2]);
            l.end(&mut self.bufs[i * mbc + 1]);
            c.end(&mut self.bufs[i * mbc]);
        }
    }

    fn mix_mono_enhanced(&mut self, out_: &mut [BlipSample], frames: i32) {
        let mv = self.max_voices as usize;
        let mbc = MAX_BUF_COUNT as usize;
        for i in 0..mv {
            let mut out = 0usize;
            let bass = self.bufs[i * mbc + 2].bass_shift_;
            let (b_center, b_sq1, b_sq2) = (
                &self.bufs[i * mbc + 2],
                &self.bufs[i * mbc],
                &self.bufs[i * mbc + 1],
            );
            let mut center = BlipReader::begin(b_center);
            let mut sq1 = BlipReader::begin(b_sq1);
            let mut sq2 = BlipReader::begin(b_sq2);

            let reverb_buf = &mut self.reverb_buf[i];
            let echo_buf = &mut self.echo_buf[i];
            let mut echo_pos = self.echo_pos[i];
            let mut reverb_pos = self.reverb_pos[i];
            let chans = &self.chans;

            for _ in 0..frames {
                let sum1_s = sq1.read();
                let sum2_s = sq2.read();

                sq1.next(b_sq1, bass);
                sq2.next(b_sq2, bass);

                let new_reverb_l = (fmul(sum1_s as i64, chans.pan_1_levels[0])
                    + fmul(sum2_s as i64, chans.pan_2_levels[0])
                    + reverb_buf
                        [((reverb_pos + chans.reverb_delay_l) as u32 & REVERB_MASK) as usize]
                        as i64) as i32;

                let new_reverb_r = (fmul(sum1_s as i64, chans.pan_1_levels[1])
                    + fmul(sum2_s as i64, chans.pan_2_levels[1])
                    + reverb_buf
                        [((reverb_pos + chans.reverb_delay_r) as u32 & REVERB_MASK) as usize]
                        as i64) as i32;

                let reverb_level = chans.reverb_level;
                reverb_buf[reverb_pos as usize] =
                    fmul(new_reverb_l as i64, reverb_level) as BlipSample;
                reverb_buf[reverb_pos as usize + 1] =
                    fmul(new_reverb_r as i64, reverb_level) as BlipSample;
                reverb_pos = ((reverb_pos + 2) as u32 & REVERB_MASK) as i32;

                let sum3_s = center.read();
                center.next(b_center, bass);

                let mut left = (new_reverb_l as i64
                    + sum3_s as i64
                    + fmul(
                        chans.echo_level,
                        echo_buf[((echo_pos + chans.echo_delay_l) as u32 & ECHO_MASK) as usize]
                            as i64,
                    )) as i32;
                let mut right = (new_reverb_r as i64
                    + sum3_s as i64
                    + fmul(
                        chans.echo_level,
                        echo_buf[((echo_pos + chans.echo_delay_r) as u32 & ECHO_MASK) as usize]
                            as i64,
                    )) as i32;

                echo_buf[echo_pos as usize] = sum3_s as BlipSample;
                echo_pos = ((echo_pos + 1) as u32 & ECHO_MASK) as i32;

                if left as i16 as i32 != left {
                    left = 0x7FFF - (left >> 24);
                }

                if right as i16 as i32 != right {
                    right = 0x7FFF - (right >> 24);
                }

                out_[out + i * 2] = left as i16;
                out_[out + i * 2 + 1] = right as i16;
                out += mv * 2;
            }
            self.reverb_pos[i] = reverb_pos;
            self.echo_pos[i] = echo_pos;

            sq1.end(&mut self.bufs[i * mbc]);
            sq2.end(&mut self.bufs[i * mbc + 1]);
            center.end(&mut self.bufs[i * mbc + 2]);
        }
    }

    fn mix_enhanced(&mut self, out_: &mut [BlipSample], frames: i32) {
        let mv = self.max_voices as usize;
        let mbc = MAX_BUF_COUNT as usize;
        for i in 0..mv {
            let mut out = 0usize;
            let bufs = &self.bufs[i * mbc..i * mbc + 7];
            let bass = bufs[2].bass_shift_;
            let mut center = BlipReader::begin(&bufs[2]);
            let mut l1 = BlipReader::begin(&bufs[3]);
            let mut r1 = BlipReader::begin(&bufs[4]);
            let mut l2 = BlipReader::begin(&bufs[5]);
            let mut r2 = BlipReader::begin(&bufs[6]);
            let mut sq1 = BlipReader::begin(&bufs[0]);
            let mut sq2 = BlipReader::begin(&bufs[1]);

            let reverb_buf = &mut self.reverb_buf[i];
            let echo_buf = &mut self.echo_buf[i];
            let mut echo_pos = self.echo_pos[i];
            let mut reverb_pos = self.reverb_pos[i];
            let chans = &self.chans;

            for _ in 0..frames {
                let sum1_s = sq1.read();
                let sum2_s = sq2.read();

                sq1.next(&bufs[0], bass);
                sq2.next(&bufs[1], bass);

                let new_reverb_l = (fmul(sum1_s as i64, chans.pan_1_levels[0])
                    + fmul(sum2_s as i64, chans.pan_2_levels[0])
                    + l1.read() as i64
                    + reverb_buf
                        [((reverb_pos + chans.reverb_delay_l) as u32 & REVERB_MASK) as usize]
                        as i64) as i32;

                let new_reverb_r = (fmul(sum1_s as i64, chans.pan_1_levels[1])
                    + fmul(sum2_s as i64, chans.pan_2_levels[1])
                    + r1.read() as i64
                    + reverb_buf
                        [((reverb_pos + chans.reverb_delay_r) as u32 & REVERB_MASK) as usize]
                        as i64) as i32;

                l1.next(&bufs[3], bass);
                r1.next(&bufs[4], bass);

                let reverb_level = chans.reverb_level;
                reverb_buf[reverb_pos as usize] =
                    fmul(new_reverb_l as i64, reverb_level) as BlipSample;
                reverb_buf[reverb_pos as usize + 1] =
                    fmul(new_reverb_r as i64, reverb_level) as BlipSample;
                reverb_pos = ((reverb_pos + 2) as u32 & REVERB_MASK) as i32;

                let sum3_s = center.read();
                center.next(&bufs[2], bass);

                let mut left = (new_reverb_l as i64
                    + sum3_s as i64
                    + l2.read() as i64
                    + fmul(
                        chans.echo_level,
                        echo_buf[((echo_pos + chans.echo_delay_l) as u32 & ECHO_MASK) as usize]
                            as i64,
                    )) as i32;
                let mut right = (new_reverb_r as i64
                    + sum3_s as i64
                    + r2.read() as i64
                    + fmul(
                        chans.echo_level,
                        echo_buf[((echo_pos + chans.echo_delay_r) as u32 & ECHO_MASK) as usize]
                            as i64,
                    )) as i32;

                l2.next(&bufs[5], bass);
                r2.next(&bufs[6], bass);

                echo_buf[echo_pos as usize] = sum3_s as BlipSample;
                echo_pos = ((echo_pos + 1) as u32 & ECHO_MASK) as i32;

                if left as i16 as i32 != left {
                    left = 0x7FFF - (left >> 24);
                }

                if right as i16 as i32 != right {
                    right = 0x7FFF - (right >> 24);
                }

                out_[out + i * 2] = left as i16;
                out_[out + i * 2 + 1] = right as i16;

                out += mv * 2;
            }
            self.reverb_pos[i] = reverb_pos;
            self.echo_pos[i] = echo_pos;

            let bufs = &mut self.bufs[i * mbc..i * mbc + 7];
            l1.end(&mut bufs[3]);
            r1.end(&mut bufs[4]);
            l2.end(&mut bufs[5]);
            r2.end(&mut bufs[6]);
            sq1.end(&mut bufs[0]);
            sq2.end(&mut bufs[1]);
            center.end(&mut bufs[2]);
        }
    }
}

impl MultiBuffer for EffectsBuffer {
    fn base(&self) -> &MultiBufferBase {
        &self.base
    }

    fn bufs(&mut self) -> &mut [BlipBuffer] {
        &mut self.bufs
    }

    fn as_effects_buffer(&mut self) -> Option<&mut EffectsBuffer> {
        Some(self)
    }

    fn set_sample_rate(&mut self, rate: i64, msec: i32) -> BlarggErr {
        // (the echo and reverb buffers are allocated in new())

        for b in self.bufs.iter_mut() {
            b.set_sample_rate(rate, msec)?;
        }

        let c = self.config_;
        self.config(&c);
        self.clear();

        let (r, l) = (self.bufs[0].sample_rate(), self.bufs[0].length());
        self.base.set_sample_rate(r, l)
    }

    fn clock_rate(&mut self, rate: u32) {
        for b in self.bufs.iter_mut() {
            b.set_clock_rate(rate);
        }
    }

    fn bass_freq(&mut self, freq: i32) {
        for b in self.bufs.iter_mut() {
            b.bass_freq(freq);
        }
    }

    fn clear(&mut self) {
        self.stereo_remain = 0;
        self.effect_remain = 0;

        for i in 0..self.max_voices as usize {
            self.echo_buf[i].fill(0);
            self.reverb_buf[i].fill(0);
        }

        for b in self.bufs.iter_mut() {
            b.clear(true);
        }
    }

    fn channel(&self, i: i32, type_: i32) -> Channel {
        let mut out = CHAN_TYPES_COUNT - 1;
        if type_ == 0 {
            out = i % 5;
            if out > CHAN_TYPES_COUNT - 1 {
                out = CHAN_TYPES_COUNT - 1;
            }
        } else if type_ & NOISE_TYPE == 0 && (type_ & TYPE_INDEX_MASK) % 3 != 0 {
            out = type_ & 1;
        }
        self.chan_types[((i % self.max_voices) * CHAN_TYPES_COUNT + out) as usize]
    }

    fn end_frame(&mut self, clock_count: BlipTime) {
        let mut bufs_used = 0;
        let stereo_mask = if self.config_.effects_enabled {
            0x78
        } else {
            0x06
        };

        let buf_count_per_voice = self.buf_count / self.max_voices;
        for v in 0..self.max_voices {
            // foreach voice
            for i in 0..buf_count_per_voice {
                // foreach buffer of that voice
                let b = &mut self.bufs[(v * buf_count_per_voice + i) as usize];
                bufs_used |= b.clear_modified() << i;
                b.end_frame(clock_count);

                if bufs_used & stereo_mask != 0 && self.buf_count == self.max_voices * MAX_BUF_COUNT
                {
                    self.stereo_remain = self
                        .stereo_remain
                        .max(b.samples_avail() + b.output_latency() as i64);
                }
                if self.effects_enabled || self.config_.effects_enabled {
                    self.effect_remain = self
                        .effect_remain
                        .max(b.samples_avail() + b.output_latency() as i64);
                }
            }
            bufs_used = 0;
        }

        self.effects_enabled = self.config_.effects_enabled;
    }

    fn samples_avail(&self) -> i64 {
        self.bufs[0].samples_avail() * 2
    }

    fn read_samples(&mut self, out: &mut [BlipSample], total_samples: i64) -> i64 {
        let n_channels = (self.max_voices * 2) as i64;
        let buf_count_per_voice = self.buf_count / self.max_voices;

        // require( total_samples % n_channels == 0 ); // as many items needed to fill at least one frame

        let mut remain = self.bufs[0].samples_avail();
        remain = remain.min(total_samples / n_channels);
        let total_samples = remain;

        let mut pos = 0usize;
        while remain != 0 {
            let mut active_bufs = buf_count_per_voice;
            let mut count = remain;

            // optimizing mixing to skip any channels which had nothing added
            if self.effect_remain != 0 {
                if count > self.effect_remain {
                    count = self.effect_remain;
                }

                if self.stereo_remain != 0 {
                    self.mix_enhanced(&mut out[pos..], count as i32);
                } else {
                    self.mix_mono_enhanced(&mut out[pos..], count as i32);
                    active_bufs = 3;
                }
            } else if self.stereo_remain != 0 {
                self.mix_stereo(&mut out[pos..], count as i32);
                active_bufs = 3;
            } else {
                self.mix_mono(&mut out[pos..], count as i32);
                active_bufs = 1;
            }

            pos += (count * n_channels) as usize;
            remain -= count;

            self.stereo_remain -= count;
            if self.stereo_remain < 0 {
                self.stereo_remain = 0;
            }

            self.effect_remain -= count;
            if self.effect_remain < 0 {
                self.effect_remain = 0;
            }

            // skip the output from any buffers that didn't contribute to the sound output
            // during this frame (e.g. if we only render mono then only the very first buf
            // is 'active')
            for v in 0..self.max_voices {
                // foreach voice
                for i in 0..buf_count_per_voice {
                    // foreach buffer of that voice
                    let b = &mut self.bufs[(v * buf_count_per_voice + i) as usize];
                    if i < active_bufs {
                        b.remove_samples(count);
                    } else {
                        // keep time synchronized
                        b.remove_silence(count);
                    }
                }
            }
        }

        total_samples * n_channels
    }
}
