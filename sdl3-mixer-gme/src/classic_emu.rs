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

//! Common aspects of emulators which use Blip_Buffer for sound output
//! (`Classic_Emu.h`, `Classic_Emu.cpp`), and the ROM data handler several
//! of them use (`Rom_Data`).
//!
//! The [`ClassicEmu`] trait holds `Classic_Emu`'s virtual functions; its
//! overrides of `Music_Emu`'s are the `classic_*` functions here, which the
//! emulators' [`MusicEmu`] implementations call.

use crate::blargg_common::{try_resize, BlarggErr};
use crate::blip_buffer::{BlipEq, BlipTime, BufId};
use crate::data_reader::DataReader;
use crate::gme_file::GME_WRONG_FILE_TYPE;
use crate::multi_buffer::{MultiBuffer, StereoBuffer};
use crate::music_emu::{music_emu_start_track_, Equalizer, MusicEmu, Sample};

pub const WAVE_TYPE: i32 = 0x100;
pub const NOISE_TYPE: i32 = 0x200;
pub const MIXED_TYPE: i32 = WAVE_TYPE | NOISE_TYPE;

/// The fields of `Classic_Emu`.
#[derive(Default)]
pub struct ClassicEmuState {
    /// (`buf`: the buffer set with `set_buffer()`, or the `stereo_buffer`
    /// made when none was)
    pub buf: Option<Box<dyn MultiBuffer>>,
    /// whether `buf` came from `set_buffer()`
    custom_buf: bool,
    clock_rate_: u32,
    buf_changed_count: u32,
    voice_types: Option<&'static [i32]>,
}

impl std::fmt::Debug for ClassicEmuState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassicEmuState")
            .field("clock_rate", &self.clock_rate_)
            .finish_non_exhaustive()
    }
}

impl ClassicEmuState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The buffers of the multi-channel buffer, for synthesis.
    pub fn bufs(&mut self) -> &mut [crate::blip_buffer::BlipBuffer] {
        self.buf
            .as_mut()
            .expect("buffer set up by set_sample_rate()")
            .bufs()
    }

    // Services
    pub fn set_voice_types(&mut self, t: &'static [i32]) {
        self.voice_types = Some(t);
    }

    pub fn clock_rate(&self) -> i64 {
        self.clock_rate_ as i64
    }

    /// experimental
    pub fn change_clock_rate(&mut self, rate: u32) {
        self.clock_rate_ = rate;
        if let Some(b) = self.buf.as_mut() {
            b.clock_rate(rate);
        }
    }

    /// `Classic_Emu::set_buffer()`
    pub fn set_buffer(&mut self, new_buf: Box<dyn MultiBuffer>) {
        // assert( !buf && new_buf );
        self.buf = Some(new_buf);
        self.custom_buf = true;
    }

    /// The `Effects_Buffer` set with `set_buffer()`, if any.
    pub fn effects_buffer(&mut self) -> Option<&mut crate::effects_buffer::EffectsBuffer> {
        if self.custom_buf {
            self.buf.as_mut().and_then(|b| b.as_effects_buffer())
        } else {
            None
        }
    }
}

/// `Classic_Emu`'s virtual functions.
pub trait ClassicEmu: MusicEmu {
    fn classic(&self) -> &ClassicEmuState;
    fn classic_mut(&mut self) -> &mut ClassicEmuState;

    // Overridable
    fn set_voice(
        &mut self,
        index: i32,
        center: Option<BufId>,
        left: Option<BufId>,
        right: Option<BufId>,
    );
    fn update_eq(&mut self, eq: &BlipEq);
    fn run_clocks(&mut self, time_io: &mut BlipTime, msec: i32) -> BlarggErr;
}

/// `Classic_Emu::set_equalizer_()`
pub fn classic_set_equalizer_<E: ClassicEmu + ?Sized>(this: &mut E, eq: &Equalizer) {
    // Music_Emu::set_equalizer_( eq );
    this.update_eq(&BlipEq::new(eq.treble));
    let bass = this.equalizer().bass as i32;
    if let Some(b) = this.classic_mut().buf.as_mut() {
        b.bass_freq(bass);
    }
}

/// `Classic_Emu::set_sample_rate_()`
pub fn classic_set_sample_rate_<E: ClassicEmu + ?Sized>(this: &mut E, rate: i64) -> BlarggErr {
    let c = this.classic_mut();
    if c.buf.is_none() {
        c.buf = Some(Box::new(StereoBuffer::new()));
    }
    c.buf.as_mut().unwrap().set_sample_rate(rate, 1000 / 20)
}

/// `Classic_Emu::set_multi_channel()`
pub fn classic_set_multi_channel<E: ClassicEmu + ?Sized>(
    this: &mut E,
    is_enabled: bool,
) -> BlarggErr {
    this.emu_mut().set_multi_channel_(is_enabled)?;
    Ok(())
}

/// `Classic_Emu::mute_voices_()`
pub fn classic_mute_voices_<E: ClassicEmu + ?Sized>(this: &mut E, mask: i32) {
    // Music_Emu::mute_voices_( mask );
    let mut i = this.voice_count();
    while i > 0 {
        i -= 1;
        if mask & 1i32.wrapping_shl(i as u32) != 0 {
            this.set_voice(i, None, None, None);
        } else {
            let c = this.classic();
            // FIXME (upstream): emulators with more voices than voice types
            // (NSF with several sound chips) read past the end of their
            // table; this gives those voices type 0, as upstream's patched
            // build for the tests does.
            let type_ = match c.voice_types {
                Some(t) => t.get(i as usize).copied().unwrap_or(0),
                None => 0,
            };
            let ch = c.buf.as_ref().unwrap().channel(i, type_);
            // assert( (ch.center && ch.left && ch.right) ||
            //         (!ch.center && !ch.left && !ch.right) ); // all or nothing
            this.set_voice(i, ch.center, ch.left, ch.right);
        }
    }
}

/// `Classic_Emu::setup_buffer()`
pub fn setup_buffer<E: ClassicEmu + ?Sized>(this: &mut E, rate: u32) -> BlarggErr {
    this.classic_mut().change_clock_rate(rate);
    let n = this.voice_count();
    this.classic_mut()
        .buf
        .as_mut()
        .unwrap()
        .set_channel_count(n)?;
    let eq = this.equalizer();
    this.set_equalizer(&eq);
    let c = this.classic_mut();
    c.buf_changed_count = c.buf.as_ref().unwrap().channels_changed_count();
    Ok(())
}

/// `Classic_Emu::start_track_()`
pub fn classic_start_track_<E: ClassicEmu + ?Sized>(this: &mut E, track: i32) -> BlarggErr {
    music_emu_start_track_(this, track)?;
    this.classic_mut().buf.as_mut().unwrap().clear();
    Ok(())
}

/// `Classic_Emu::play_()`
pub fn classic_play_<E: ClassicEmu + ?Sized>(this: &mut E, out: &mut [Sample]) -> BlarggErr {
    let count = out.len() as i64;
    let mut remain = count;
    while remain != 0 {
        let start = (count - remain) as usize;
        remain -= this
            .classic_mut()
            .buf
            .as_mut()
            .unwrap()
            .read_samples(&mut out[start..], remain);
        if remain != 0 {
            let changed = this
                .classic()
                .buf
                .as_ref()
                .unwrap()
                .channels_changed_count();
            if this.classic().buf_changed_count != changed {
                this.classic_mut().buf_changed_count = changed;
                this.remute_voices();
            }
            let msec = this.classic().buf.as_ref().unwrap().length();
            let mut clocks_emulated =
                ((msec as u32).wrapping_mul(this.classic().clock_rate_) / 1000) as BlipTime;
            this.run_clocks(&mut clocks_emulated, msec)?;
            // assert( clocks_emulated );
            this.classic_mut()
                .buf
                .as_mut()
                .unwrap()
                .end_frame(clocks_emulated);
        }
    }
    Ok(())
}

/// The [`MusicEmu`] methods a classic emulator overrides with
/// `Classic_Emu`'s (and the state accessors, for a type whose
/// `MusicEmuState` is its `state` field and `ClassicEmuState` its `classic`
/// field). `start_track_` is left to the emulator, which overrides it too.
macro_rules! classic_emu_overrides {
    () => {
        fn emu(&self) -> &$crate::music_emu::MusicEmuState {
            &self.state
        }
        fn emu_mut(&mut self) -> &mut $crate::music_emu::MusicEmuState {
            &mut self.state
        }
        fn set_buffer(&mut self, buf: Box<dyn $crate::multi_buffer::MultiBuffer>) {
            self.classic.set_buffer(buf);
        }
        fn effects_buffer(&mut self) -> Option<&mut $crate::effects_buffer::EffectsBuffer> {
            self.classic.effects_buffer()
        }
        fn set_multi_channel(&mut self, is_enabled: bool) -> $crate::blargg_common::BlarggErr {
            $crate::classic_emu::classic_set_multi_channel(self, is_enabled)
        }
        fn set_sample_rate_(&mut self, rate: i64) -> $crate::blargg_common::BlarggErr {
            $crate::classic_emu::classic_set_sample_rate_(self, rate)
        }
        fn mute_voices_(&mut self, mask: i32) {
            $crate::classic_emu::classic_mute_voices_(self, mask)
        }
        fn set_equalizer_(&mut self, eq: &$crate::music_emu::Equalizer) {
            $crate::classic_emu::classic_set_equalizer_(self, eq)
        }
        fn play_(
            &mut self,
            out: &mut [$crate::music_emu::Sample],
        ) -> $crate::blargg_common::BlarggErr {
            $crate::classic_emu::classic_play_(self, out)
        }
    };
}
pub(crate) use classic_emu_overrides;

// ROM data handler, used by several Classic_Emu derivitives. Loads file data
// with padding on both sides, allowing direct use in bank mapping. The main purpose
// is to allow all file data to be loaded with only one read() call (for efficiency).

const PAD_EXTRA: i32 = 8;

/// `Rom_Data<unit>` (with `Rom_Data_`)
#[derive(Debug, Default)]
pub struct RomData {
    unit: i32,
    pub rom: Vec<u8>,
    file_size_: i64,
    rom_addr: i32,
    mask: i32,
    size_: i32, // TODO: eliminate
}

impl RomData {
    pub fn new(unit: i32) -> Self {
        RomData {
            unit,
            ..Default::default()
        }
    }

    fn pad_size(&self) -> i64 {
        (self.unit + PAD_EXTRA) as i64
    }

    /// Load file data, using already-loaded header 'h' if not NULL. Copy header
    /// from loaded file data into *out and fill unmapped bytes with 'fill'.
    pub fn load(
        &mut self,
        input: &mut dyn DataReader,
        header_size: i32,
        header_out: &mut [u8],
        fill: u8,
    ) -> BlarggErr {
        let pad_size = self.pad_size();
        let file_offset = pad_size - header_size as i64;

        self.rom_addr = 0;
        self.mask = 0;
        self.size_ = 0;
        self.rom = Vec::new();

        self.file_size_ = input.remain();
        if self.file_size_ <= header_size as i64 {
            // <= because there must be data after header
            return Err(GME_WRONG_FILE_TYPE);
        }
        let mut err = try_resize(
            &mut self.rom,
            (file_offset + self.file_size_ + pad_size) as usize,
        );
        if err.is_ok() {
            let (a, b) = (
                file_offset as usize,
                (file_offset + self.file_size_) as usize,
            );
            err = input.read(&mut self.rom[a..b]);
        }
        if let Err(e) = err {
            self.rom = Vec::new();
            return Err(e);
        }

        self.file_size_ -= header_size as i64;
        let fo = file_offset as usize;
        header_out[..header_size as usize]
            .copy_from_slice(&self.rom[fo..fo + header_size as usize]);

        let len = self.rom.len();
        self.rom[..pad_size as usize].fill(fill);
        self.rom[len - pad_size as usize..].fill(fill);

        Ok(())
    }

    /// Size of file data read in (excluding header)
    pub fn file_size(&self) -> i64 {
        self.file_size_
    }

    /// Index of beginning of file data (`begin()`)
    pub fn begin(&self) -> usize {
        self.pad_size() as usize
    }

    /// Set address that file data should start at
    pub fn set_addr(&mut self, mut addr: i64) {
        let unit = self.unit as i64;
        self.rom_addr = (addr - unit - PAD_EXTRA as i64) as i32;

        let mut rounded = (addr + self.file_size_ + unit - 1) / unit * unit;
        if rounded <= 0 {
            rounded = 0;
        } else {
            let mut shift = 0;
            let max_addr = (rounded - 1) as u64;
            while max_addr.checked_shr(shift).unwrap_or(0) != 0 {
                shift += 1;
            }
            self.mask = ((1i64 << shift) - 1) as i32;
        }

        if addr < 0 {
            addr = 0;
        }
        let _ = addr;
        self.size_ = rounded as i32;
        // (upstream's realloc leaves any new space uninitialized; this zeroes it)
        let _ = try_resize(
            &mut self.rom,
            (rounded - self.rom_addr as i64 + PAD_EXTRA as i64) as usize,
        ); // OK if shrink fails
    }

    /// Free data
    pub fn clear(&mut self) {
        self.rom = Vec::new();
    }

    /// Size of data + start addr, rounded to a multiple of unit
    pub fn size(&self) -> i64 {
        self.size_ as i64
    }

    /// Index of unmapped page filled with same value (`unmapped()`)
    pub fn unmapped(&self) -> usize {
        0
    }

    /// Mask address to nearest power of two greater than size()
    pub fn mask_addr(&self, addr: i32) -> i32 {
        addr & self.mask
    }

    /// Index of page starting at addr. Returns unmapped() if outside data.
    pub fn at_addr(&self, addr: i32) -> usize {
        let mut offset = (self.mask_addr(addr) as u32).wrapping_sub(self.rom_addr as u32);
        if offset > (self.rom.len() as u64).wrapping_sub(self.pad_size() as u64) as u32 {
            offset = 0; // unmapped
        }
        offset as usize
    }
}
