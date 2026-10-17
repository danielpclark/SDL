/*
    libmpg123: MPEG Audio Decoder library (version 1.33.7)

    copyright 1995-2023 by the mpg123 project
    free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
*/
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! # sdl3-mixer-mpg123 — SDL_mixer's libmpg123, translated to Rust
//!
//! A line-by-line translation of the libmpg123 1.33.7 that SDL_mixer 3.3.0
//! vendors (`external/mpg123`), which
//! [`sdl3-mixer`](https://crates.io/crates/sdl3-mixer) uses for its
//! `MPG123` MP3 decoder: MPEG 1, 2 and 2.5 audio, layers I, II and III,
//! read through caller-supplied I/O callbacks.
//!
//! Unlike the rest of this repository, which is under the zlib license,
//! libmpg123 is under the GNU LGPL, version 2.1 (see `COPYING` and
//! `NOTICE` in this crate). `sdl3-mixer` uses it through its `mpg123`
//! Cargo feature, which is on by default; build `sdl3-mixer` with
//! `default-features = false` for a build without it (MP3 then goes to the
//! zlib-licensed dr_mp3 decoder, as in an upstream build without
//! libmpg123).
//!
//! ## What is translated
//!
//! What SDL_mixer's build of libmpg123 (its CMake port, as SDL_mixer's
//! CMakeLists configures it: static, no libout123 or programs, float
//! decoding, accurate rounding, IEEE 754 float tricks, gapless decoding,
//! a 1000-entry seek index, ID3v2 and ICY parsing, all output encodings)
//! reaches from SDL_mixer's `decoder_mpg123.c`, which opens a handle with
//! `mpg123_new(NULL)`, sets `MPG123_QUIET`, reads through
//! `mpg123_reader64()`, enables every standard rate in mono and stereo for
//! S8, U8, S16, S32 and F32, and decodes, scans and seeks:
//!
//! - the frame parser with free format, sync, resync and junk skipping,
//!   ID3v1, ID3v2 (stored, as `mpg123_id3()` returns it), APE and RIFF
//!   headers, and the Xing/Info/LAME header with the gapless information;
//! - the layer I, II and III decoders, the plain C (`generic`) synth for
//!   the 16-bit, 8-bit, float and 32-bit outputs, and the output
//!   conversions;
//! - the stream reader over the I/O callbacks, the frame index and exact
//!   seeking (and fuzzy seeking, with `MPG123_FUZZY`).
//!
//! Not translated, as SDL_mixer can't reach them: the assembly decoders
//! (upstream's x86-64 build picks its SSE or AVX decoder where the CPU has
//! it; this is the `generic` decoder it falls back to) and the dithered
//! `generic_dither` one, which only a decoder name asks for; the 2:1 and
//! 4:1 downsampling and the NtoM resampling synths (libmpg123 only uses
//! them when the native rate isn't enabled, and SDL_mixer enables all; this
//! behaves as a build with `NO_DOWNSAMPLE` and `NO_NTOM` there); the feeder
//! and buffered readers (`NO_FEEDER`); the equalizer API; the
//! `lfs_wrap.c` file-name and file-descriptor I/O; and `icy2utf8.c` (under
//! a BSD-style license, only used by `mpg123_icy2utf8()`). libmpg123 1.33
//! has no network code.
//!
//! The API is upstream's `mpg123_*` functions as methods of
//! [`Mpg123Handle`], with upstream's return codes; each names the C
//! function it translates.

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
    clippy::eq_op,
    clippy::explicit_counter_loop,
    clippy::erasing_op,
    clippy::excessive_precision,
    clippy::identity_op,
    clippy::if_same_then_else,
    clippy::manual_clamp,
    clippy::manual_is_multiple_of,
    clippy::manual_memcpy,
    clippy::manual_range_contains,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::needless_return,
    clippy::single_match,
    clippy::too_many_arguments,
    clippy::unnecessary_cast,
    clippy::unusual_byte_groupings
)]

mod costabs;
mod dct64;
mod format;
mod frame;
mod getbits;
mod icy;
mod id3;
mod index;
mod l12tabs;
mod l2tables;
mod l3bandgain;
mod l3tabs;
mod layer1;
mod layer2;
mod layer3;
mod libmpg123;
pub mod mpg123;
mod newhuffman;
mod optimize;
mod parse;
mod readers;
mod stringbuf;
mod synth;
mod tabinit;

pub use format::{mpg123_encodings, mpg123_encsize, mpg123_rates};
pub use frame::Mpg123Handle;
pub use id3::{Id3Link, Mpg123Id3v1, Mpg123Id3v2, Mpg123Picture, Mpg123Text};
pub use libmpg123::{
    mpg123_exit, mpg123_init, mpg123_plain_strerror, mpg123_safe_buffer, Mpg123FrameInfo,
};
pub use readers::{Mpg123Io, SEEK_CUR, SEEK_END, SEEK_SET};
pub use stringbuf::{mpg123_strlen, Mpg123String};
