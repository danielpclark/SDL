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

//! Combination of Fir_Resampler and Blip_Buffer mixing. Used by Sega FM emulators.
//! (`Dual_Resampler.h`, `Dual_Resampler.cpp`)

use crate::blargg_common::{try_resize, BlarggErr};
use crate::blip_buffer::{BlipBuffer, BlipReader, BlipTime};
use crate::fir_resampler::FirResampler;

pub type DSample = i16;

/// `Dual_Resampler`'s virtual function, and the `Blip_Buffer` its
/// `dual_play()` is given: what the emulator using it provides.
pub trait DualResamplerHost {
    fn play_frame(&mut self, blip_time: BlipTime, pcm_count: i32, pcm_out: &mut [DSample]) -> i32;
    /// The `Blip_Buffer&` given to `dual_play()`
    fn blip_buf(&mut self) -> &mut BlipBuffer;
}

/// `Dual_Resampler`
#[derive(Clone, Debug)]
pub struct DualResampler {
    sample_buf: Vec<DSample>,
    sample_buf_size: i32,
    oversamples_per_frame: i32,
    buf_pos: i32,
    resampler_size: i32,
    resampler: FirResampler,
}

impl Default for DualResampler {
    fn default() -> Self {
        Self::new()
    }
}

impl DualResampler {
    pub fn new() -> Self {
        DualResampler {
            sample_buf: Vec::new(),
            sample_buf_size: 0,
            oversamples_per_frame: -1,
            buf_pos: -1,
            resampler_size: 0,
            resampler: FirResampler::new(12),
        }
    }

    pub fn setup(&mut self, oversample: f64, rolloff: f64, gain: f64) -> f64 {
        self.resampler.time_ratio(oversample, rolloff, gain * 0.5)
    }

    pub fn reset(&mut self, pairs: i32) -> BlarggErr {
        // expand allocations a bit
        try_resize(&mut self.sample_buf, ((pairs + (pairs >> 2)) * 2) as usize)?;
        self.resize(pairs);
        self.resampler_size = self.oversamples_per_frame + (self.oversamples_per_frame >> 2);
        self.resampler.buffer_size(self.resampler_size)
    }

    pub fn resize(&mut self, pairs: i32) {
        let new_sample_buf_size = pairs * 2;
        if self.sample_buf_size != new_sample_buf_size {
            if new_sample_buf_size as u32 as usize > self.sample_buf.len() {
                // check( false );
                return;
            }
            self.sample_buf_size = new_sample_buf_size;
            self.oversamples_per_frame = (pairs as f64 * self.resampler.ratio()) as i32 * 2 + 2;
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.buf_pos = self.sample_buf_size;
        self.resampler.clear();
    }

    /// `play_frame_()`: into `out`, or (`None`) into `sample_buf`
    fn play_frame_<H: DualResamplerHost + ?Sized>(
        &mut self,
        host: &mut H,
        out: Option<&mut [DSample]>,
    ) {
        let pair_count = (self.sample_buf_size >> 1) as i64;
        let blip_time = host.blip_buf().count_clocks(pair_count);
        let sample_count = self.oversamples_per_frame - self.resampler.written();

        let new_count = host.play_frame(blip_time, sample_count, self.resampler.buffer());
        // assert( new_count < resampler_size );

        host.blip_buf().end_frame(blip_time);
        // assert( blip_buf.samples_avail() == pair_count );

        self.resampler.write(new_count as i64);

        // (NDEBUG: the count read is not checked)
        let n = self.sample_buf_size;
        self.resampler.read(&mut self.sample_buf, n);

        self.mix_samples(host.blip_buf(), out);
        host.blip_buf().remove_samples(pair_count);
    }

    pub fn dual_play<H: DualResamplerHost + ?Sized>(
        &mut self,
        mut count: i64,
        out: &mut [DSample],
        host: &mut H,
    ) {
        let mut o = 0usize;

        // empty extra buffer
        let mut remain = (self.sample_buf_size - self.buf_pos) as i64;
        if remain != 0 {
            if remain > count {
                remain = count;
            }
            count -= remain;
            let (from, n) = (self.buf_pos as usize, remain as usize);
            out[o..o + n].copy_from_slice(&self.sample_buf[from..from + n]);
            o += n;
            self.buf_pos += remain as i32;
        }

        // entire frames
        while count >= self.sample_buf_size as i64 {
            self.play_frame_(host, Some(&mut out[o..]));
            o += self.sample_buf_size as usize;
            count -= self.sample_buf_size as i64;
        }

        // extra
        if count != 0 {
            self.play_frame_(host, None);
            self.buf_pos = count as i32;
            let n = count as usize;
            out[o..o + n].copy_from_slice(&self.sample_buf[..n]);
        }
    }

    fn mix_samples(&mut self, blip_buf: &mut BlipBuffer, out: Option<&mut [DSample]>) {
        let bass = blip_buf.bass_shift_;
        let mut sn = BlipReader::begin(blip_buf);
        let pairs = (self.sample_buf_size >> 1) as usize;
        let mut mix = |input: [DSample; 2]| -> [DSample; 2] {
            let s = sn.read();
            let mut l: i32 = input[0] as i32 * 2 + s;
            if l as i16 as i32 != l {
                l = 0x7FFF - (l >> 24);
            }
            sn.next(blip_buf, bass);
            let mut r: i32 = input[1] as i32 * 2 + s;
            if r as i16 as i32 != r {
                r = 0x7FFF - (r >> 24);
            }
            [l as DSample, r as DSample]
        };
        match out {
            Some(out) => {
                for n in 0..pairs {
                    let i = n * 2;
                    let lr = mix([self.sample_buf[i], self.sample_buf[i + 1]]);
                    out[i] = lr[0];
                    out[i + 1] = lr[1];
                }
            }
            None => {
                // (in place: `out` is `sample_buf`)
                for n in 0..pairs {
                    let i = n * 2;
                    let lr = mix([self.sample_buf[i], self.sample_buf[i + 1]]);
                    self.sample_buf[i] = lr[0];
                    self.sample_buf[i + 1] = lr[1];
                }
            }
        }
        sn.end(blip_buf);
    }
}
