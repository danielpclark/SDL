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

//! Band-limited sound synthesis buffer (`Blip_Buffer.h`, `Blip_Buffer.cpp`).
//!
//! The synths keep the buffer they write to as an index into the buffers
//! of the [`MultiBuffer`](crate::multi_buffer::MultiBuffer) that owns them
//! (upstream keeps a `Blip_Buffer*`), so the functions that synthesize
//! take that set of buffers.

use crate::blargg_common::BlarggErr;
use sdl3::stdlib::math::{cos, floor, pow, sin};

// internal
pub type BlipLong = i32;
pub type BlipUlong = u32;

/// Time unit at source clock rate
pub type BlipTime = BlipLong;

/// Output samples are 16-bit signed, with a range of -32768 to 32767
pub type BlipSample = i16;
pub const BLIP_SAMPLE_MAX: i32 = 32767;

pub type BlipResampledTime = BlipUlong;

/// An index into the buffers of a multi-channel buffer (upstream's
/// `Blip_Buffer*`); `None` is upstream's NULL.
pub type BufId = usize;

/// Number of bits in resample ratio fraction. Higher values give a more accurate ratio
/// but reduce maximum buffer size.
pub const BLIP_BUFFER_ACCURACY: u32 = 16;

/// Number bits in phase offset. Fewer than 6 bits (64 phase offsets) results in
/// noticeable broadband noise when synthesizing high frequency square waves.
/// Affects size of Blip_Synth objects since they store the waveform directly.
pub const BLIP_PHASE_BITS: u32 = 6;

// Internal
pub const BLIP_WIDEST_IMPULSE_: i32 = 16;
pub const BLIP_BUFFER_EXTRA_: i32 = BLIP_WIDEST_IMPULSE_ + 2;
pub const BLIP_RES: i32 = 1 << BLIP_PHASE_BITS;

// Quality level. Start with blip_good_quality.
pub const BLIP_MED_QUALITY: i32 = 8;
pub const BLIP_GOOD_QUALITY: i32 = 12;
pub const BLIP_HIGH_QUALITY: i32 = 16;

pub const BLIP_SAMPLE_BITS: i32 = 30;

/// Constant value to use instead of BLIP_READER_BASS(), for slightly more optimal
/// code at the cost of having no bass control
pub const BLIP_READER_DEFAULT_BASS: i32 = 9;

pub const BLIP_MAX_LENGTH: i32 = 0;
pub const BLIP_DEFAULT_LENGTH: i32 = 250;

pub struct BlipBuffer {
    pub factor_: BlipUlong,
    pub offset_: BlipResampledTime,
    pub buffer_: Vec<BlipLong>,
    pub buffer_size_: BlipLong,
    pub reader_accum_: BlipLong,
    pub bass_shift_: i32,
    sample_rate_: i64,
    clock_rate_: u32,
    bass_freq_: i32,
    length_: i32,
    modified_: i32,
}

impl std::fmt::Debug for BlipBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlipBuffer")
            .field("sample_rate", &self.sample_rate_)
            .field("clock_rate", &self.clock_rate_)
            .field("length", &self.length_)
            .finish_non_exhaustive()
    }
}

impl Default for BlipBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl BlipBuffer {
    pub fn new() -> Self {
        BlipBuffer {
            factor_: BlipUlong::MAX / 2,
            offset_: 0,
            buffer_: Vec::new(),
            buffer_size_: 0,
            sample_rate_: 0,
            reader_accum_: 0,
            bass_shift_: 0,
            clock_rate_: 0,
            bass_freq_: 16,
            length_: 0,
            // (upstream leaves it uninitialized until clear())
            modified_: 0,
        }
    }

    /// Remove all available samples and clear buffer to silence. If 'entire_buffer' is
    /// false, just clears out any samples waiting rather than the entire buffer.
    pub fn clear(&mut self, entire_buffer: bool) {
        self.offset_ = 0;
        self.reader_accum_ = 0;
        self.modified_ = 0;
        if !self.buffer_.is_empty() {
            let count = if entire_buffer {
                self.buffer_size_ as i64
            } else {
                self.samples_avail()
            };
            let n = (count + BLIP_BUFFER_EXTRA_ as i64) as usize;
            self.buffer_[..n].fill(0);
        }
    }

    /// Set output sample rate and buffer length in milliseconds (1/1000 sec, defaults
    /// to 1/4 second), then clear buffer. Returns NULL on success, otherwise if there
    /// isn't enough memory, returns error without affecting current buffer setup.
    pub fn set_sample_rate(&mut self, new_rate: i64, msec: i32) -> BlarggErr {
        // start with maximum length that resampled time can represent
        let mut new_size: i64 =
            (u32::MAX >> BLIP_BUFFER_ACCURACY) as i64 - BLIP_BUFFER_EXTRA_ as i64 - 64;
        if msec != BLIP_MAX_LENGTH {
            let s = (new_rate * (msec as i64 + 1) + 999) / 1000;
            if s < new_size {
                new_size = s;
            }
            // else: assert( 0 ); fails if requested buffer length exceeds limit
        }

        if self.buffer_size_ as i64 != new_size {
            let n = (new_size + BLIP_BUFFER_EXTRA_ as i64) as usize;
            if n > self.buffer_.len() {
                if self
                    .buffer_
                    .try_reserve_exact(n - self.buffer_.len())
                    .is_err()
                {
                    return Err("Out of memory");
                }
            }
            // (realloc's new space is uninitialized; clear() below zeroes it)
            self.buffer_.resize(n, 0);
        }

        self.buffer_size_ = new_size as BlipLong;

        // update things based on the sample rate
        self.sample_rate_ = new_rate;
        self.length_ = (new_size * 1000 / new_rate - 1) as i32;
        // if ( msec ) assert( length_ == msec ); // ensure length is same as that passed in
        if self.clock_rate_ != 0 {
            let r = self.clock_rate_;
            self.set_clock_rate(r);
        }
        let b = self.bass_freq_;
        self.bass_freq(b);

        self.clear(true);

        Ok(()) // success
    }

    pub fn clock_rate_factor(&self, rate: u32) -> BlipResampledTime {
        let ratio = self.sample_rate_ as f64 / rate as f64;
        let factor = floor(ratio * (1i64 << BLIP_BUFFER_ACCURACY) as f64 + 0.5) as BlipLong;
        // assert( factor > 0 || !sample_rate_ ); // fails if clock/output ratio is too large
        factor as BlipResampledTime
    }

    /// Set frequency high-pass filter frequency, where higher values reduce bass more
    pub fn bass_freq(&mut self, freq: i32) {
        self.bass_freq_ = freq;
        let mut shift = 31;
        if freq > 0 {
            shift = 13;
            let mut f: i64 = ((freq.wrapping_shl(16)) as i64) / self.sample_rate_;
            loop {
                f >>= 1;
                if f == 0 {
                    break;
                }
                shift -= 1;
                if shift == 0 {
                    break;
                }
            }
        }
        self.bass_shift_ = shift;
    }

    /// End current time frame of specified duration and make its samples available
    /// (along with any still-unread samples) for reading with read_samples(). Begins
    /// a new time frame at the end of the current frame.
    pub fn end_frame(&mut self, t: BlipTime) {
        self.offset_ = self
            .offset_
            .wrapping_add((t as u32).wrapping_mul(self.factor_));
    }

    pub fn remove_silence(&mut self, count: i64) {
        // assert( count <= samples_avail() ); // tried to remove more samples than available
        self.offset_ = self
            .offset_
            .wrapping_sub((count as BlipResampledTime).wrapping_shl(BLIP_BUFFER_ACCURACY));
    }

    /// Number of raw samples that can be mixed within frame of specified duration.
    pub fn count_samples(&self, t: BlipTime) -> i64 {
        let last_sample = (self.resampled_time(t) >> BLIP_BUFFER_ACCURACY) as u64;
        let first_sample = (self.offset_ >> BLIP_BUFFER_ACCURACY) as u64;
        last_sample.wrapping_sub(first_sample) as i64
    }

    /// Count number of clocks needed until 'count' samples will be available.
    /// If buffer can't even hold 'count' samples, returns number of clocks until
    /// buffer becomes full.
    pub fn count_clocks(&self, mut count: i64) -> BlipTime {
        if self.factor_ == 0 {
            // assert( 0 ); // sample rate and clock rates must be set first
            return 0;
        }

        if count > self.buffer_size_ as i64 {
            count = self.buffer_size_ as i64;
        }
        let time = (count as BlipResampledTime).wrapping_shl(BLIP_BUFFER_ACCURACY);
        (time
            .wrapping_sub(self.offset_)
            .wrapping_add(self.factor_)
            .wrapping_sub(1)
            / self.factor_) as BlipTime
    }

    /// Remove 'count' samples from those waiting to be read
    pub fn remove_samples(&mut self, count: i64) {
        if count != 0 {
            self.remove_silence(count);

            // copy remaining samples to beginning and clear old samples
            let remain = (self.samples_avail() + BLIP_BUFFER_EXTRA_ as i64) as usize;
            let count = count as usize;
            self.buffer_.copy_within(count..count + remain, 0);
            self.buffer_[remain..remain + count].fill(0);
        }
    }

    /// Read at most 'max_samples' out of buffer into 'dest', removing them from from
    /// the buffer. Returns number of samples actually read and removed. If stereo is
    /// true, increments 'dest' one extra time after writing each sample, to allow
    /// easy interleving of two channels into a stereo output buffer.
    pub fn read_samples(&mut self, out: &mut [BlipSample], max_samples: i64, stereo: bool) -> i64 {
        let mut count = self.samples_avail();
        if count > max_samples {
            count = max_samples;
        }

        if count != 0 {
            let bass = self.bass_shift_;
            let mut accum = self.reader_accum_;
            let step = if stereo { 2 } else { 1 };
            for n in 0..count as usize {
                let mut s = accum >> (BLIP_SAMPLE_BITS - 16);
                if s as BlipSample as i32 != s {
                    s = 0x7FFF - (s >> 24);
                }
                out[n * step] = s as BlipSample;
                accum = accum.wrapping_add(self.buffer_[n].wrapping_sub(accum >> bass));
            }
            self.reader_accum_ = accum;

            self.remove_samples(count);
        }
        count
    }

    /// Mix 'count' samples from 'buf' into buffer.
    pub fn mix_samples(&mut self, input: &[BlipSample]) {
        let mut out =
            (self.offset_ >> BLIP_BUFFER_ACCURACY) as usize + (BLIP_WIDEST_IMPULSE_ / 2) as usize;

        let sample_shift = BLIP_SAMPLE_BITS - 16;
        let mut prev: i32 = 0;
        for &s in input {
            let s = (s as BlipLong) << sample_shift;
            self.buffer_[out] = self.buffer_[out].wrapping_add(s.wrapping_sub(prev));
            prev = s;
            out += 1;
        }
        self.buffer_[out] = self.buffer_[out].wrapping_sub(prev);
    }

    // not documented yet
    pub fn set_modified(&mut self) {
        self.modified_ = 1;
    }
    pub fn clear_modified(&mut self) -> i32 {
        let b = self.modified_;
        self.modified_ = 0;
        b
    }
    pub fn resampled_duration(&self, t: i32) -> BlipResampledTime {
        (t as u32).wrapping_mul(self.factor_)
    }
    pub fn resampled_time(&self, t: BlipTime) -> BlipResampledTime {
        (t as u32)
            .wrapping_mul(self.factor_)
            .wrapping_add(self.offset_)
    }

    /// Length of buffer, in milliseconds
    pub fn length(&self) -> i32 {
        self.length_
    }
    /// Number of samples available for reading with read_samples()
    pub fn samples_avail(&self) -> i64 {
        let samples = (self.offset_ >> BLIP_BUFFER_ACCURACY) as i64;
        if samples <= self.buffer_size_ as i64 {
            samples
        } else {
            0
        }
    }
    /// Current output sample rate
    pub fn sample_rate(&self) -> i64 {
        self.sample_rate_
    }
    /// Number of samples delay from synthesis to samples read out
    pub fn output_latency(&self) -> i32 {
        BLIP_WIDEST_IMPULSE_ / 2
    }
    /// Number of source time units per second
    pub fn clock_rate(&self) -> u32 {
        self.clock_rate_
    }
    /// Set number of source time units per second
    pub fn set_clock_rate(&mut self, cps: u32) {
        self.clock_rate_ = cps;
        self.factor_ = self.clock_rate_factor(cps);
    }
}

/// `BLIP_READER_*`: optimized reading from a Blip_Buffer, for use in custom
/// sample output. `begin()` is `BLIP_READER_BEGIN()`, `read()`
/// `BLIP_READER_READ()`, `read_raw()` `BLIP_READER_READ_RAW()`, `next()`
/// `BLIP_READER_NEXT()` and `end()` `BLIP_READER_END()`.
#[derive(Clone, Copy, Debug)]
pub struct BlipReader {
    pos: usize,
    accum: BlipLong,
}

impl BlipReader {
    pub fn begin(b: &BlipBuffer) -> Self {
        BlipReader {
            pos: 0,
            accum: b.reader_accum_,
        }
    }
    /// Current sample
    #[inline]
    pub fn read(&self) -> BlipLong {
        self.accum >> (BLIP_SAMPLE_BITS - 16)
    }
    /// Current raw sample in full internal resolution
    #[inline]
    pub fn read_raw(&self) -> BlipLong {
        self.accum
    }
    /// Advance to next sample
    #[inline]
    pub fn next(&mut self, b: &BlipBuffer, bass: i32) {
        self.accum = self
            .accum
            .wrapping_add(b.buffer_[self.pos].wrapping_sub(self.accum >> bass));
        self.pos += 1;
    }
    /// End reading samples from buffer. The number of samples read must now be removed
    /// using Blip_Buffer::remove_samples().
    pub fn end(self, b: &mut BlipBuffer) {
        b.reader_accum_ = self.accum;
    }
}

/// Low-pass equalization parameters
#[derive(Clone, Copy, Debug)]
pub struct BlipEq {
    treble: f64,
    rolloff_freq: i64,
    sample_rate: i64,
    cutoff_freq: i64,
}

impl BlipEq {
    /// Logarithmic rolloff to treble dB at half sampling rate. Negative values reduce
    /// treble, small positive values (0 to 5.0) increase treble.
    pub fn new(treble_db: f64) -> Self {
        BlipEq {
            treble: treble_db,
            rolloff_freq: 0,
            sample_rate: 44100,
            cutoff_freq: 0,
        }
    }

    /// See blip_buffer.txt
    pub fn with_rolloff(
        treble: f64,
        rolloff_freq: i64,
        sample_rate: i64,
        cutoff_freq: i64,
    ) -> Self {
        BlipEq {
            treble,
            rolloff_freq,
            sample_rate,
            cutoff_freq,
        }
    }

    fn generate(&self, out: &mut [f32], count: i32) {
        // lower cutoff freq for narrow kernels with their wider transition band
        // (8 points->1.49, 16 points->1.15)
        let mut oversample = BLIP_RES as f64 * 2.25 / count as f64 + 0.85;
        let half_rate = self.sample_rate as f64 * 0.5;
        if self.cutoff_freq != 0 {
            oversample = half_rate / self.cutoff_freq as f64;
        }
        let cutoff = self.rolloff_freq as f64 * oversample / half_rate;

        gen_sinc(
            out,
            count,
            BLIP_RES as f64 * oversample,
            self.treble,
            cutoff,
        );

        // apply (half of) hamming window
        let to_fraction = PI / (count - 1) as f64;
        let mut i = count;
        while i > 0 {
            i -= 1;
            out[i as usize] *= 0.54f32 - 0.46f32 * cos(i as f64 * to_fraction) as f32;
        }
    }
}

const PI: f64 = 3.1415926535897932384626433832795029;

fn gen_sinc(out: &mut [f32], count: i32, oversample: f64, mut treble: f64, mut cutoff: f64) {
    if cutoff >= 0.999 {
        cutoff = 0.999;
    }

    if treble < -300.0 {
        treble = -300.0;
    }
    if treble > 5.0 {
        treble = 5.0;
    }

    let maxh = 4096.0;
    let rolloff = pow(10.0, 1.0 / (maxh * 20.0) * treble / (1.0 - cutoff));
    let pow_a_n = pow(rolloff, maxh - maxh * cutoff);
    let to_angle = PI / 2.0 / maxh / oversample;
    for i in 0..count {
        let angle = (((i - count) * 2 + 1) as f64) * to_angle;
        let angle_maxh = angle * maxh;
        let angle_maxh_mid = angle_maxh * cutoff;

        let mut y = maxh;

        // 0 to Fs/2*cutoff, flat
        if angle_maxh_mid != 0.0 {
            // unstable at t=0
            y *= sin(angle_maxh_mid) / angle_maxh_mid;
        }

        // Fs/2*cutoff to Fs/2, logarithmic rolloff
        let cosa = cos(angle);
        let den = 1.0 + rolloff * (rolloff - cosa - cosa);

        // Becomes unstable when rolloff is near 1.0 and t is near 0,
        // which is the only time den becomes small
        if den > 1e-13 {
            let num = (cos(angle_maxh - angle) * rolloff - cos(angle_maxh)) * pow_a_n
                - cos(angle_maxh_mid - angle) * rolloff
                + cos(angle_maxh_mid);

            y = y * cutoff + num / den;
        }

        out[i as usize] = y as f32;
    }
}

/// `Blip_Synth<quality,range>` with `Blip_Synth_` (the impulses are kept
/// for the widest quality).
///
/// Range specifies the greatest expected change in amplitude. Calculate it
/// by finding the difference between the maximum and minimum expected
/// amplitudes (max - min).
#[derive(Clone)]
pub struct BlipSynth {
    quality: i32,
    range: i32,
    /// The buffer `update()` and `offset()` write to (`impl.buf`).
    pub buf: Option<BufId>,
    pub last_amp: i32,
    pub delta_factor: i32,
    volume_unit_: f64,
    width: i32,
    kernel_unit: BlipLong,
    impulses: [i16; (BLIP_RES * (BLIP_HIGH_QUALITY / 2) + 1) as usize],
}

impl std::fmt::Debug for BlipSynth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlipSynth")
            .field("quality", &self.quality)
            .field("range", &self.range)
            .finish_non_exhaustive()
    }
}

impl BlipSynth {
    pub fn new(quality: i32, range: i32) -> Self {
        BlipSynth {
            quality,
            range,
            buf: None,
            last_amp: 0,
            delta_factor: 0,
            volume_unit_: 0.0,
            width: quality,
            kernel_unit: 0,
            // (upstream leaves them uninitialized until treble_eq())
            impulses: [0; (BLIP_RES * (BLIP_HIGH_QUALITY / 2) + 1) as usize],
        }
    }

    /// Set overall volume of waveform
    pub fn volume(&mut self, v: f64) {
        let range = if self.range < 0 {
            -self.range
        } else {
            self.range
        };
        self.volume_unit(v * (1.0 / range as f64));
    }

    /// Get/set Blip_Buffer used for output
    pub fn output(&self) -> Option<BufId> {
        self.buf
    }
    pub fn set_output(&mut self, b: Option<BufId>) {
        self.buf = b;
        self.last_amp = 0;
    }

    fn impulses_size(&self) -> i32 {
        BLIP_RES / 2 * self.width + 1
    }

    fn adjust_impulse(&mut self) {
        // sum pairs for each phase and add error correction to end of first half
        let size = self.impulses_size();
        let mut p = BLIP_RES;
        loop {
            let cond = p >= BLIP_RES / 2;
            p -= 1;
            if !cond {
                break;
            }
            let p2 = BLIP_RES - 2 - p;
            let mut error: i64 = self.kernel_unit as i64;
            let mut i = 1;
            while i < size {
                error -= self.impulses[(i + p) as usize] as i64;
                error -= self.impulses[(i + p2) as usize] as i64;
                i += BLIP_RES;
            }
            if p == p2 {
                error /= 2; // phase = 0.5 impulse uses same half for both sides
            }
            let idx = (size - BLIP_RES + p) as usize;
            self.impulses[idx] = self.impulses[idx].wrapping_add(error as i16);
            //printf( "error: %ld\n", error );
        }

        //for ( int i = blip_res; i--; printf( "\n" ) )
        //  for ( int j = 0; j < width / 2; j++ )
        //      printf( "%5ld,", impulses [j * blip_res + i + 1] );
    }

    /// Configure low-pass filter (see blip_buffer.txt)
    pub fn treble_eq(&mut self, eq: &BlipEq) {
        let mut fimpulse =
            [0f32; (BLIP_RES / 2 * (BLIP_WIDEST_IMPULSE_ - 1) + BLIP_RES * 2) as usize];

        let half_size = BLIP_RES / 2 * (self.width - 1);
        eq.generate(&mut fimpulse[BLIP_RES as usize..], half_size);

        // need mirror slightly past center for calculation
        let mut i = BLIP_RES;
        while i > 0 {
            i -= 1;
            fimpulse[(BLIP_RES + half_size + i) as usize] =
                fimpulse[(BLIP_RES + half_size - 1 - i) as usize];
        }

        // starts at 0
        for i in 0..BLIP_RES {
            fimpulse[i as usize] = 0.0;
        }

        // find rescale factor
        let mut total = 0.0f64;
        for i in 0..half_size {
            total += fimpulse[(BLIP_RES + i) as usize] as f64;
        }

        //double const base_unit = 44800.0 - 128 * 18; // allows treble up to +0 dB
        //double const base_unit = 37888.0; // allows treble to +5 dB
        let base_unit = 32768.0f64; // necessary for blip_unscaled to work
        let rescale = base_unit / 2.0 / total;
        self.kernel_unit = base_unit as BlipLong;

        // integrate, first difference, rescale, convert to int
        let mut sum = 0.0f64;
        let mut next = 0.0f64;
        let impulses_size = self.impulses_size();
        for i in 0..impulses_size {
            self.impulses[i as usize] = floor((next - sum) * rescale + 0.5) as i32 as i16;
            sum += fimpulse[i as usize] as f64;
            next += fimpulse[(i + BLIP_RES) as usize] as f64;
        }
        self.adjust_impulse();

        // volume might require rescaling
        let vol = self.volume_unit_;
        if vol != 0.0 {
            self.volume_unit_ = 0.0;
            self.volume_unit(vol);
        }
    }

    pub fn volume_unit(&mut self, new_unit: f64) {
        if new_unit != self.volume_unit_ {
            // use default eq if it hasn't been set yet
            if self.kernel_unit == 0 {
                self.treble_eq(&BlipEq::new(-8.0));
            }

            self.volume_unit_ = new_unit;
            let mut factor = new_unit * (1i64 << BLIP_SAMPLE_BITS) as f64 / self.kernel_unit as f64;

            if factor > 0.0 {
                let mut shift = 0;

                // if unit is really small, might need to attenuate kernel
                while factor < 2.0 {
                    shift += 1;
                    factor *= 2.0;
                }

                if shift != 0 {
                    self.kernel_unit >>= shift;
                    // assert( kernel_unit > 0 ); // fails if volume unit is too low

                    // keep values positive to avoid round-towards-zero of sign-preserving
                    // right shift for negative values
                    let offset: i64 = 0x8000 + (1i64 << (shift - 1));
                    let offset2: i64 = 0x8000 >> shift;
                    let mut i = self.impulses_size();
                    while i > 0 {
                        i -= 1;
                        let v = self.impulses[i as usize] as i64;
                        self.impulses[i as usize] = (((v + offset) >> shift) - offset2) as i16;
                    }
                    self.adjust_impulse();
                }
            }
            self.delta_factor = floor(factor + 0.5) as i32;
            //printf( "delta_factor: %d, kernel_unit: %d\n", delta_factor, kernel_unit );
        }
    }

    /// Works directly in terms of fractional output samples. Contact author for more info.
    #[inline]
    pub fn offset_resampled(&self, time: BlipResampledTime, delta: i32, blip_buf: &mut BlipBuffer) {
        // Fails if time is beyond end of Blip_Buffer, due to a bug in caller code or the
        // need for a longer buffer as set by set_sample_rate().
        // assert( (blip_long) (time >> BLIP_BUFFER_ACCURACY) < blip_buf->buffer_size_ );
        let delta = delta.wrapping_mul(self.delta_factor);
        let base = (time >> BLIP_BUFFER_ACCURACY) as usize;
        let phase =
            ((time >> (BLIP_BUFFER_ACCURACY - BLIP_PHASE_BITS)) & (BLIP_RES as u32 - 1)) as usize;

        let quality = self.quality as usize;
        let fwd = (BLIP_WIDEST_IMPULSE_ as usize - quality) / 2;
        let half = quality / 2;
        // FIXME (upstream): a time past the end of the buffer (which the
        // assertion above catches in debug builds) writes past it; this
        // leaves the buffer alone then.
        let Some(buf) = blip_buf.buffer_.get_mut(base + fwd..base + fwd + quality) else {
            return;
        };
        let res = BLIP_RES as usize;

        // straight forward implementation resulted in better code on GCC for x86
        // (the first half, BLIP_FWD(), with imp = impulses + blip_res - phase...)
        let imp = &self.impulses[res - phase..];
        for k in 0..half {
            buf[k] = buf[k].wrapping_add((imp[res * k] as BlipLong).wrapping_mul(delta));
        }
        // (...and the second, BLIP_REV(), with imp = impulses + phase)
        let imp = &self.impulses[phase..];
        for k in 0..half {
            buf[half + k] = buf[half + k]
                .wrapping_add((imp[res * (half - 1 - k)] as BlipLong).wrapping_mul(delta));
        }
    }

    /// Add an amplitude transition of specified delta, optionally into specified buffer
    /// rather than the one set with output(). Delta can be positive or negative.
    /// The actual change in amplitude is delta * (volume / range)
    #[inline]
    pub fn offset(&self, t: BlipTime, delta: i32, buf: &mut BlipBuffer) {
        self.offset_resampled(
            (t as u32)
                .wrapping_mul(buf.factor_)
                .wrapping_add(buf.offset_),
            delta,
            buf,
        );
    }

    /// Same as offset(), except code is inlined for higher performance
    #[inline]
    pub fn offset_inline(&self, t: BlipTime, delta: i32, buf: &mut BlipBuffer) {
        self.offset(t, delta, buf);
    }

    /// Update amplitude of waveform at given time. Using this requires a separate
    /// Blip_Synth for each waveform.
    pub fn update(&mut self, t: BlipTime, amp: i32, bufs: &mut [BlipBuffer]) {
        let delta = amp.wrapping_sub(self.last_amp);
        self.last_amp = amp;
        if let Some(b) = self.buf {
            self.offset(t, delta, &mut bufs[b]);
        }
    }
}
