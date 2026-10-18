// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2004-2006 Shay Green. This module is free software; you
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

//! Finite impulse response (FIR) resampler with adjustable FIR size
//! (`Fir_Resampler.h`, `Fir_Resampler.cpp`). `Fir_Resampler<width>` is
//! [`FirResampler`] made with its width.

use crate::blargg_common::{try_resize, BlarggErr};
use sdl3::stdlib::math::{cos, floor, pow};

pub type Sample = i16;

const STEREO: i32 = 2;
const MAX_RES: i32 = 32;

const PI: f64 = 3.1415926535897932384626433832795029;

fn gen_sinc(
    rolloff: f64,
    width: i32,
    offset: f64,
    spacing: f64,
    mut scale: f64,
    mut count: i32,
    out: &mut [i16],
) {
    let maxh = 256.0;
    let step = PI / maxh * spacing;
    let to_w = maxh * 2.0 / width as f64;
    let pow_a_n = pow(rolloff, maxh);
    scale /= maxh * 2.0;

    let mut angle = ((count / 2 - 1) as f64 + offset) * -step;
    let mut o = 0;
    while count > 0 {
        count -= 1;
        out[o] = 0;
        o += 1;
        let w = angle * to_w;
        if w.abs() < PI {
            let rolloff_cos_a = rolloff * cos(angle);
            let num = 1.0 - rolloff_cos_a - pow_a_n * cos(maxh * angle)
                + pow_a_n * rolloff * cos((maxh - 1.0) * angle);
            let den = 1.0 - rolloff_cos_a - rolloff_cos_a + rolloff * rolloff;
            let sinc = scale * num / den - scale;

            out[o - 1] = ((cos(w) * sinc + sinc) as i32) as i16;
        }
        angle += step;
    }
}

/// `Fir_Resampler_` with `Fir_Resampler<width>`'s impulses
#[derive(Clone, Debug)]
pub struct FirResampler {
    buf: Vec<Sample>,
    write_pos: usize,
    res: i32,
    imp_phase: i32,
    width_: i32,
    write_offset: i32,
    skip_bits: u32,
    step: i32,
    input_per_cycle: i32,
    ratio_: f64,
    /// `impulses [max_res] [width]`
    impulses: Vec<i16>,
}

impl FirResampler {
    /// Width is number of points in FIR. Must be even and 4 or more. More points give
    /// better quality and rolloff effectiveness, and take longer to calculate.
    pub fn new(width: i32) -> Self {
        // static_assert( width >= 4 && width % 2 == 0, ... );
        FirResampler {
            buf: Vec::new(),
            write_pos: 0,
            res: 1,
            imp_phase: 0,
            width_: width,
            write_offset: width * STEREO - STEREO,
            skip_bits: 0,
            step: STEREO,
            input_per_cycle: 0,
            ratio_: 1.0,
            // (upstream leaves them uninitialized until time_ratio())
            impulses: vec![0; (MAX_RES * width) as usize],
        }
    }

    /// Set input/output resampling ratio and optionally low-pass rolloff and gain.
    /// Returns actual ratio used (rounded to internal precision).
    pub fn time_ratio(&mut self, new_factor: f64, rolloff: f64, gain: f64) -> f64 {
        self.ratio_ = new_factor;

        let mut fstep = 0.0;
        {
            let mut least_error = 2.0;
            let mut pos = 0.0;
            self.res = -1;
            for r in 1..=MAX_RES {
                pos += self.ratio_;
                let nearest = floor(pos + 0.5);
                let error = (pos - nearest).abs();
                if error < least_error {
                    self.res = r;
                    fstep = nearest / self.res as f64;
                    least_error = error;
                }
            }
        }

        self.skip_bits = 0;

        self.step = STEREO * floor(fstep) as i32;

        self.ratio_ = fstep;
        fstep %= 1.0;

        let filter = if self.ratio_ < 1.0 {
            1.0
        } else {
            1.0 / self.ratio_
        };
        let mut pos = 0.0;
        self.input_per_cycle = 0;
        let width = self.width_;
        for i in 0..self.res {
            let at = (i * width) as usize;
            gen_sinc(
                rolloff,
                (width as f64 * filter + 1.0) as i32 & !1,
                pos,
                filter,
                0x7FFF as f64 * gain * filter,
                width,
                &mut self.impulses[at..at + width as usize],
            );

            pos += fstep;
            self.input_per_cycle += self.step;
            if pos >= 0.9999999 {
                pos -= 1.0;
                self.skip_bits |= 1 << i;
                self.input_per_cycle += 1;
            }
        }

        self.clear();

        self.ratio_
    }

    /// Current input/output ratio
    pub fn ratio(&self) -> f64 {
        self.ratio_
    }

    // Input

    /// Resize and clear input buffer
    pub fn buffer_size(&mut self, new_size: i32) -> BlarggErr {
        try_resize(&mut self.buf, (new_size + self.write_offset) as usize)?;
        self.clear();
        Ok(())
    }

    /// Clear input buffer. At least two output samples will be available after
    /// two input samples are written.
    pub fn clear(&mut self) {
        self.imp_phase = 0;
        if !self.buf.is_empty() {
            self.write_pos = self.write_offset as usize;
            let n = self.write_offset as usize;
            self.buf[..n].fill(0);
        }
    }

    /// Number of input samples that can be written
    pub fn max_write(&self) -> i32 {
        (self.buf.len() - self.write_pos) as i32
    }

    /// Place to write input samples (`buffer()`: the samples from it)
    pub fn buffer(&mut self) -> &mut [Sample] {
        &mut self.buf[self.write_pos..]
    }

    /// Notify resampler that 'count' input samples have been written
    pub fn write(&mut self, count: i64) {
        self.write_pos += count as usize;
        // assert( write_pos <= buf.end() );
    }

    /// Number of input samples in buffer
    pub fn written(&self) -> i32 {
        self.write_pos as i32 - self.write_offset
    }

    /// Skip 'count' input samples. Returns number of samples actually skipped.
    pub fn skip_input(&mut self, mut count: i64) -> i32 {
        let mut remain = self.write_pos as i32;
        let mut max_count = remain - self.width_ * STEREO;
        if max_count < 0 {
            max_count = 0;
        }
        if count > max_count as i64 {
            count = max_count as i64;
        }

        remain -= count as i32;
        self.write_pos = remain as usize;
        let c = count as usize;
        self.buf.copy_within(c..c + remain as usize, 0);

        count as i32
    }

    // Output

    /// Number of extra input samples needed until 'count' output samples are available
    pub fn input_needed(&self, mut output_count: i32) -> i32 {
        let mut input_count: i32 = 0;

        let mut skip: u64 = (self.skip_bits >> self.imp_phase) as u64;
        let mut remain = self.res - self.imp_phase;
        loop {
            output_count -= 2;
            if output_count <= 0 {
                break;
            }
            input_count += self.step + (skip & 1) as i32 * STEREO;
            skip >>= 1;
            remain -= 1;
            if remain == 0 {
                skip = self.skip_bits as u64;
                remain = self.res;
            }
            output_count -= 2;
        }

        let mut input_extra =
            input_count as i64 - (self.write_pos as i64 - ((self.width_ - 1) * STEREO) as i64);
        if input_extra < 0 {
            input_extra = 0;
        }
        input_extra as i32
    }

    /// Number of output samples available
    pub fn avail(&self) -> i32 {
        self.avail_(self.write_pos as i32 - self.width_ * STEREO)
    }

    fn avail_(&self, mut input_count: i32) -> i32 {
        let cycle_count = input_count / self.input_per_cycle;
        let mut output_count = cycle_count * self.res * STEREO;
        input_count -= cycle_count * self.input_per_cycle;

        let mut skip: u32 = self.skip_bits >> self.imp_phase;
        let mut remain = self.res - self.imp_phase;
        while input_count >= 0 {
            input_count -= self.step + (skip & 1) as i32 * STEREO;
            skip >>= 1;
            remain -= 1;
            if remain == 0 {
                skip = self.skip_bits;
                remain = self.res;
            }
            output_count += 2;
        }
        output_count
    }

    /// Read at most 'count' samples. Returns number of samples actually read.
    pub fn read(&mut self, out: &mut [Sample], mut count: i32) -> i32 {
        let width = self.width_ as usize;
        let mut o = 0usize;
        let mut input = 0usize;
        let mut end_pos = self.write_pos;
        let mut skip: u32 = self.skip_bits >> self.imp_phase;
        let mut imp = (self.imp_phase * self.width_) as usize;
        let mut remain = self.res - self.imp_phase;
        let step = self.step as usize;

        count >>= 1;

        // Resampling can add noise so don't actually do it if we've matched sample
        // rate
        let ratio1 = self.ratio() - 1.0;
        let should_resample = (if ratio1 >= 0.0 { ratio1 } else { -ratio1 }) >= 0.00001;

        if end_pos as i64 - input as i64 >= (width as i64) * STEREO as i64 {
            end_pos -= width * STEREO as usize;
            loop {
                count -= 1;
                if count < 0 {
                    break;
                }

                if !should_resample {
                    out[o] = self.buf[input];
                    out[o + 1] = self.buf[input + 1];
                } else {
                    // accumulate in extended precision
                    let mut l: i32 = 0;
                    let mut r: i32 = 0;

                    let mut i = input;

                    for _ in 0..width / 2 {
                        let pt0 = self.impulses[imp] as i32;
                        l = l.wrapping_add(pt0 * self.buf[i] as i32);
                        r = r.wrapping_add(pt0 * self.buf[i + 1] as i32);
                        let pt1 = self.impulses[imp + 1] as i32;
                        imp += 2;
                        l = l.wrapping_add(pt1 * self.buf[i + 2] as i32);
                        r = r.wrapping_add(pt1 * self.buf[i + 3] as i32);
                        i += 4;
                    }

                    remain -= 1;

                    l >>= 15;
                    r >>= 15;

                    input += ((skip * STEREO as u32) & STEREO as u32) as usize;
                    skip >>= 1;

                    if remain == 0 {
                        imp = 0;
                        skip = self.skip_bits;
                        remain = self.res;
                    }

                    out[o] = l as Sample;
                    out[o + 1] = r as Sample;
                }

                input += step;
                o += 2;
                if input > end_pos {
                    break;
                }
            }
        }

        self.imp_phase = self.res - remain;

        // Note (upstream): at a ratio of 24 or more, a step can go past the
        // end of the input, leaving a negative count of samples; this
        // leaves none, and upstream's build for the tests is patched the
        // same.
        if input > self.write_pos {
            input = self.write_pos;
        }
        let left = self.write_pos - input;
        self.write_pos = left;
        self.buf.copy_within(input..input + left, 0);

        o as i32
    }
}
