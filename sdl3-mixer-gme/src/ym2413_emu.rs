// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/
// Modified 2026-10-18: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! YM2413 FM sound chip emulator interface (`Ym2413_Emu.h`, and the
//! `Ym2413_Emu.cpp` SDL_mixer's build has: "Use in place of Ym2413_Emu.cpp
//! and ym2413.c to disable support for this chip").

/// `Ym2413_Emu`: without an emulator (`opll` is left out, unused).
#[derive(Debug, Default)]
pub struct Ym2413Emu {}

impl Ym2413Emu {
    // Mute voice n if bit n (1 << n) of mask is set
    pub const CHANNEL_COUNT: i32 = 14;
    // stereo
    pub const OUT_CHAN_COUNT: i32 = 2;

    pub fn new() -> Self {
        Ym2413Emu {}
    }

    /// Set output sample rate and chip clock rates, in Hz. Returns non-zero
    /// if error.
    pub fn set_rate(&mut self, _sample_rate: f64, _clock_rate: f64) -> i32 {
        2
    }

    /// Reset to power-up state
    pub fn reset(&mut self) {}

    /// Write 'data' to 'addr'
    pub fn write(&mut self, _addr: i32, _data: i32) {}

    /// Mute voice n if bit n (1 << n) of mask is set
    pub fn mute_voices(&mut self, _mask: i32) {}

    /// Run and write pair_count samples to output
    pub fn run(&mut self, _pair_count: i32, _out: &mut [i16]) {}
}
