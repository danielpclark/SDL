// Game_Music_Emu 0.6.5: Game Music Emulators
// Copyright (C) 2003-2006 Shay Green, and its other authors; see NOTICE.
// Game_Music_Emu is free software; you can redistribute it and/or modify
// it under the terms of the GNU Lesser General Public License as published
// by the Free Software Foundation; either version 2.1 of the License, or
// (at your option) any later version (see COPYING).
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! # sdl3-mixer-gme — SDL_mixer's Game_Music_Emu, translated to Rust
//!
//! A line-by-line translation of the Game_Music_Emu 0.6.5 that SDL_mixer
//! 3.3.0 vendors (`external/libgme`, its `v0.6.5-SDL` branch), for
//! SDL_mixer's `GME` decoder: the music of classic video game consoles and
//! computers, emulated from the rips of their sound programs.
//!
//! Unlike the rest of this repository, which is under the zlib license,
//! Game_Music_Emu is under the GNU LGPL, version 2.1 or later (see
//! `COPYING` and `NOTICE` in this crate), and the emu2413 YM2413 emulator
//! it carries for the NES VRC7 is under the MIT license
//! (`COPYING.emu2413`).
//!
//! ## What is translated
//!
//! What SDL_mixer's build of libgme (its CMake project, as SDL_mixer's
//! CMakeLists configures it: every emulator, the Nuked OPN2 YM2612 core,
//! no zlib) reaches from SDL_mixer's `decoder_gme.c`, which identifies a
//! file by its header, opens it from memory for its information and to
//! play it, starts and switches tracks, plays and seeks:
//!
//! - `Music_Emu` and `Gme_File` (track starting, silence detection and
//!   skipping, fades, seeking, tempo, voice muting, the equalizer), the
//!   track information, `Classic_Emu` with its ROM handler, the readers;
//! - `Blip_Buffer` (band-limited synthesis, its synths and equalization),
//!   `Stereo_Buffer` and `Effects_Buffer` (with its stereo depth, echo and
//!   reverb);
//! - the AY emulator: the Z80 CPU (its two upstream copies, `Ay_Cpu` and
//!   `Kss_Cpu`, are one translation over each emulator's memory), the
//!   AY-3-8910 and the ZX Spectrum's beeper;
//! - the GBS emulator: the Game Boy CPU and its APU;
//! - the HES emulator: the PC Engine's HuC6280 CPU, its PSG and the ADPCM
//!   chip;
//! - the KSS emulator: the MSX's banked memory, Konami's SCC, and the
//!   Sega Master System's SN76489;
//! - the NSF and NSFE emulators: the NES 6502 CPU, the 2A03 APU and the
//!   VRC6, Namco 106, FME-7, FDS, MMC5 and VRC7 sound chips (the VRC7 over
//!   emu2413);
//! - the SAP emulator: the Atari XL's 6502 CPU and its POKEY;
//! - the SPC emulator: the SPC-700 CPU, the (fast) S-DSP, the output filter
//!   and the FIR resampler.
//!
//! The API is upstream's `gme_*` functions (in [`gme`]), on [`MusicEmu`]
//! trait objects. `sdl3-mixer` uses it for its `GME` decoder through its
//! `gme` Cargo feature, on by default (`default-features = false` leaves
//! it out).

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
// The translation keeps upstream's code shape (its branches, constants,
// counters and arithmetic, including its quirks), which these lints would
// have rewritten.
#![allow(
    clippy::approx_constant,
    clippy::collapsible_else_if,
    clippy::collapsible_match,
    clippy::explicit_counter_loop,
    clippy::int_plus_one,
    clippy::manual_clamp,
    clippy::manual_div_ceil,
    clippy::manual_rotate,
    clippy::manual_swap,
    clippy::collapsible_if,
    clippy::comparison_chain,
    clippy::eq_op,
    clippy::erasing_op,
    clippy::excessive_precision,
    clippy::identity_op,
    clippy::if_same_then_else,
    clippy::manual_range_contains,
    clippy::needless_range_loop,
    clippy::needless_return,
    clippy::single_match,
    clippy::too_many_arguments,
    clippy::unnecessary_cast,
    clippy::new_without_default
)]

pub mod ay_apu;
pub mod ay_cpu;
pub mod ay_emu;
pub mod blargg_common;
pub mod blip_buffer;
pub mod classic_emu;
pub mod data_reader;
pub mod effects_buffer;
pub mod emu2413;
pub mod fir_resampler;
pub mod gb_apu;
pub mod gb_cpu;
pub mod gb_oscs;
pub mod gbs_emu;
pub mod gme;
pub mod gme_file;
pub mod hes_apu;
pub mod hes_apu_adpcm;
pub mod hes_cpu;
pub mod hes_emu;
pub mod kss_cpu;
pub mod kss_emu;
pub mod kss_scc_apu;
pub mod multi_buffer;
pub mod music_emu;
pub mod nes_apu;
pub mod nes_cpu;
pub mod nes_fds_apu;
pub mod nes_fme7_apu;
pub mod nes_mmc5_apu;
pub mod nes_namco_apu;
pub mod nes_oscs;
pub mod nes_vrc6_apu;
pub mod nes_vrc7_apu;
pub mod nsf_emu;
pub mod nsfe_emu;
pub mod sap_apu;
pub mod sap_cpu;
pub mod sap_emu;
pub mod sms_apu;
pub mod snes_spc;
pub mod spc_cpu;
pub mod spc_dsp;
pub mod spc_emu;
pub mod spc_filter;

pub use gme::*;
pub use gme_file::{GmeType, GmeTypeT, TrackInfo};
pub use music_emu::{Equalizer, MusicEmu};
