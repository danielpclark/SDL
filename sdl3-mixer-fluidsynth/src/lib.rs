/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: translated into Rust from fluidsynth.h,
// fluidsynth/version.h, the build configuration (config.h) as SDL_mixer
// 3.3.0's CMake generates it for its FluidSynth 2.5.7, and
// fluid_audio_driver_register() of fluid_adriver.c (see NOTICE).

//! # sdl3-mixer-fluidsynth — SDL_mixer's FluidSynth, translated to Rust
//!
//! A line-by-line translation of the parts of FluidSynth 2.5.7 (the
//! SoundFont 2 synthesizer by Peter Hanappe, Josh Green, David Henningsson,
//! Tom Moebert and others), as SDL_mixer 3.3.0 vendors it in
//! `external/fluidsynth` and builds it with its CMake, that SDL_mixer's
//! FluidSynth MIDI decoder reaches, for
//! [`sdl3-mixer`](https://crates.io/crates/sdl3-mixer). It renders
//! Standard MIDI Files with a SoundFont 2 (`.sf2`), which the caller
//! gives through file callbacks or a path. None are bundled.
//!
//! The build it matches has `double` samples (no `WITH_FLOAT`), no
//! libsndfile (so the compressed samples of SF3 SoundFonts are ignored, as
//! upstream's build without it ignores them), no mixer threads, no audio
//! or MIDI drivers, no LADSPA and no libinstpatch.
//!
//! Unlike the rest of this repository, which is under the zlib license,
//! FluidSynth is under the GNU LGPL, version 2.1 or (at your option) any
//! later version (see `COPYING` and `NOTICE` in this crate); the
//! translation of the gcem math functions that its lookup tables are
//! computed with is under the Apache License 2.0 (`LICENSE.gcem`).
//! `sdl3-mixer` uses it through its `fluidsynth` Cargo feature, which is
//! on by default; build `sdl3-mixer` with `default-features = false` for
//! a build without it.
//!
//! The API is upstream's: [`Settings`] (`fluid_settings_t`), [`Synth`]
//! (`fluid_synth_t`), [`SfLoader`] (`fluid_sfloader_t`) and [`Player`]
//! (`fluid_player_t`). Every item names the C symbol it translates.
//! Where corrupt input makes upstream's C read or write out of bounds or
//! spin forever, the translation does something defined instead; those
//! places are marked `FIXME (upstream)`.
//!
//! Otherwise it renders bit for bit what upstream's C renders, which the
//! tests check against upstream built with the same math functions as
//! sdl3's (upstream compiles its IIR filter with `-ffast-math`, whose
//! results depend on the compiler and the CPU; the translation evaluates
//! the filter as written).
//!
//! What SDL_mixer doesn't reach is left out: the audio and MIDI drivers,
//! the shell, the sequencer, the MIDI router, LADSPA, reading DLS files
//! (SDL_mixer gives FluidSynth its SoundFont through loader callbacks
//! under a name only they open; the DLS loader fails to open it, as
//! upstream's does), the system timer of the player (the `"system"`
//! `player.timing-source`), SoundFont unloading, the realtime settings
//! callbacks, and most getters and setters.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
// The translation keeps upstream's code shape (its branches, constants,
// counters and arithmetic, including its quirks), which these lints would
// have rewritten.
#![allow(
    clippy::approx_constant,
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::comparison_chain,
    clippy::excessive_precision,
    clippy::field_reassign_with_default,
    clippy::identity_op,
    clippy::if_same_then_else,
    clippy::manual_clamp,
    clippy::manual_range_contains,
    clippy::module_inception,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::new_without_default,
    clippy::too_many_arguments,
    clippy::unnecessary_cast
)]

mod fluidsynth_h;
mod fluidsynth_priv;
mod gcem;
mod gentables;

mod utils {
    pub(crate) mod conv;
    pub(crate) mod conv_tables;
    pub(crate) mod ringbuffer;
    pub(crate) mod settings;
    pub(crate) mod sys;
}

mod rvoice {
    pub(crate) mod adsr_env;
    pub(crate) mod chorus;
    pub(crate) mod dsp_tables;
    pub(crate) mod iir_filter;
    pub(crate) mod lfo;
    pub(crate) mod phase;
    pub(crate) mod rev;
    pub(crate) mod rvoice;
    pub(crate) mod rvoice_dsp;
    pub(crate) mod rvoice_event;
    pub(crate) mod rvoice_mixer;
}

mod sfloader {
    pub(crate) mod defsfont;
    pub(crate) mod dls;
    pub(crate) mod samplecache;
    pub(crate) mod sffile;
    pub(crate) mod sfont;
}

mod synth {
    pub(crate) mod chan;
    pub(crate) mod gen;
    pub(crate) mod modulator;
    pub(crate) mod synth;
    pub(crate) mod synth_monopoly;
    pub(crate) mod tuning;
    pub(crate) mod voice;
}

mod midi {
    pub(crate) mod midi;
}

#[cfg(test)]
mod tests;

pub use fluidsynth_h::*;
pub use midi::midi::{FluidMidiEvent, Player};
pub use sfloader::sfont::{SfFile, SfLoader, SfOpenCallback};
pub use synth::synth::{fluid_version, fluid_version_str, Synth};
pub use utils::settings::Settings;
pub use utils::sys::{
    fluid_default_log_function, set_log_function, FluidLogFunction, FLUID_DBG, FLUID_ERR,
    FLUID_INFO, FLUID_PANIC, FLUID_WARN, LAST_LOG_LEVEL, SEEK_CUR, SEEK_END, SEEK_SET,
};

/// The sample type of the synthesis (`fluid_real_t`: `double`, as the
/// build has no `WITH_FLOAT`).
pub(crate) type FluidReal = f64;

/// FluidSynth major version (`FLUIDSYNTH_VERSION_MAJOR`)
pub const FLUIDSYNTH_VERSION_MAJOR: i32 = 2;
/// FluidSynth minor version (`FLUIDSYNTH_VERSION_MINOR`)
pub const FLUIDSYNTH_VERSION_MINOR: i32 = 5;
/// FluidSynth micro version (`FLUIDSYNTH_VERSION_MICRO`)
pub const FLUIDSYNTH_VERSION_MICRO: i32 = 7;
/// FluidSynth version string (`FLUIDSYNTH_VERSION`)
pub const FLUIDSYNTH_VERSION: &str = "2.5.7";

/// The default SoundFont of the `synth.default-soundfont` setting
/// (`DEFAULT_SOUNDFONT` of the build's config.h).
pub const DEFAULT_SOUNDFONT: &str = "/usr/local/share/soundfonts/default.sf2";

/// Translation of `fluid_audio_driver_register()`: registers the audio
/// drivers to use. The build has none, so only `None` or an empty list
/// succeeds (#FLUID_OK); any driver name fails (#FLUID_FAILED), as
/// upstream's search for it does.
pub fn audio_driver_register(adrivers: Option<&[&str]>) -> i32 {
    match adrivers {
        None => FLUID_OK,
        Some([]) => FLUID_OK,
        /* requested driver not found, failure */
        Some(_) => FLUID_FAILED,
    }
}
