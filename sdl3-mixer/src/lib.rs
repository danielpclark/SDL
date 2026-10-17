// Rust translation of include/SDL3_mixer/SDL_mixer.h from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-mixer — SDL_mixer, translated to Rust
//!
//! The pure-Rust translation of [SDL_mixer](https://github.com/libsdl-org/SDL_mixer)
//! 3, "an audio mixer library based on the SDL library": audio files in
//! many formats decoded and mixed together on [`sdl3`]'s audio streams. It
//! is a separate crate over [`sdl3`], as SDL_mixer is a separate library
//! over SDL.
//!
//! SDL_mixer's model is a [`Mixer`] (on an audio device, or generating
//! audio into a buffer with [`Mixer::generate`]) that plays any number of
//! [`Track`]s at once. A track plays from an [`Audio`] (audio data loaded
//! and maybe predecoded once, shared between tracks), from an
//! [`IoStream`](sdl3::io::IoStream) decoded on the fly, or from an
//! [`AudioStream`](sdl3::audio::AudioStream) the app feeds. Tracks have
//! their own gain, frequency ratio, fades, loops, stereo or 3D positioning,
//! and can be tagged and arranged in [`Group`]s; callbacks see the audio
//! at every stage. [`AudioDecoder`] decodes a file without a mixer.
//!
//! * Decoders: WAV (PCM, IEEE float, mu-law, a-law, MS and IMA ADPCM, with
//!   `smpl` loops), AIFF and AIFF-C, Creative VOC, Sun/NeXT AU, MP3 (the
//!   bundled dr_mp3, and libmpg123 ahead of it: see below), Ogg Vorbis
//!   (libvorbisfile and libvorbis's decoder over libogg, as SDL_mixer
//!   vendors them, translated here under their BSD licenses, then the
//!   bundled stb_vorbis), FLAC (libFLAC's decoder,
//!   as SDL_mixer vendors it, translated here under its BSD license, then
//!   the bundled dr_flac, which also reads Ogg FLAC), Ogg Opus (opusfile,
//!   libopus's decoder and libogg,
//!   as SDL_mixer vendors them, translated here under their BSD licenses),
//!   WavPack (its library, as SDL_mixer vendors it, translated here under
//!   its BSD license: lossless and hybrid files, with their .wvc correction
//!   files, every bit depth, floats and multichannel), raw PCM and a sine
//!   wave generator; ID3v1/v2, APE,
//!   Lyrics3 and MusicMatch tags and Ogg comments become metadata
//!   properties, and Ogg `LOOPSTART`-style comments loop.
//! * MIDI (the `FLUIDSYNTH` decoder, tried first) through FluidSynth, as
//!   SDL_mixer vendors it, translated in the `sdl3-mixer-fluidsynth` crate,
//!   with a SoundFont 2 the app gives (the
//!   `SDL_mixer.decoder.fluidsynth.soundfont_iostream` or
//!   `SDL_mixer.decoder.fluidsynth.soundfont_path` load property, with
//!   FluidSynth settings in `SDL_mixer.decoder.fluidsynth.props`) or, on
//!   Unix systems but Apple's, one at `/usr/share/sounds/sf2/default-GM.sf2`
//!   and the like; without one, it plays MIDI files silently, as upstream's
//!   does. This is the `fluidsynth` Cargo feature, on by default (as
//!   upstream builds it by default). **FluidSynth is not zlib-licensed**:
//!   `sdl3-mixer-fluidsynth` is under `LGPL-2.1-or-later AND Apache-2.0`.
//! * MIDI (the `TIMIDITY` decoder) through the bundled TiMidity, translated
//!   in the `sdl3-mixer-timidity` crate, with GUS patches or a SoundFont,
//!   as the TiMidity configuration file (the `TIMIDITY_CFG` environment
//!   variable, or `/etc/timidity.cfg` and the like) or the
//!   `TIMIDITY_SOUNDFONT` environment variable says. This is the
//!   `timidity` Cargo feature, on by default (as upstream builds it by
//!   default). **TiMidity is not zlib-licensed**: `sdl3-mixer-timidity` is
//!   under `Artistic-1.0-Perl OR LGPL-2.1-only`. Build with
//!   `default-features = false` for an sdl3-mixer without MIDI (neither
//!   FluidSynth nor TiMidity), without libmpg123 and without game music
//!   (below) under the zlib license only (and the BSD licenses of its
//!   Vorbis, FLAC, Opus and
//!   WavPack decoders' libraries, and libxmp's MIT license).
//! * MP3 and MPEG audio layers I and II (the `MPG123` decoder, tried
//!   before dr_mp3's `DRMP3`) through the libmpg123 SDL_mixer vendors,
//!   translated in the `sdl3-mixer-mpg123` crate. This is the `mpg123`
//!   Cargo feature, on by default (as upstream builds it by default).
//!   **libmpg123 is not zlib-licensed**: `sdl3-mixer-mpg123` is under
//!   `LGPL-2.1-only`. With `default-features = false`, MP3 goes to dr_mp3
//!   alone, as in an upstream build without libmpg123.
//! * Tracker modules (the `XMP` decoder) through libxmp, as SDL_mixer
//!   vendors it, translated here under its MIT license: the player and its
//!   software mixer, with all 52 of the loaders SDL_mixer's build enables
//!   (MOD and its relatives, XM and OXM, S3M, IT, the MED, Oktalyzer,
//!   Farandole, Digitrakker, Ultra Tracker, Poly Tracker, Composer 669 and
//!   many more formats); the module's name, type and comment become
//!   metadata, and playback can start at an order
//!   ([`PROP_PLAY_START_ORDER_NUMBER`]).
//! * Game music (the `GME` decoder) through the Game_Music_Emu SDL_mixer
//!   vendors, translated in the `sdl3-mixer-gme` crate: AY (ZX Spectrum and
//!   Amstrad CPC), GBS (Game Boy), HES (PC Engine/TurboGrafx-16), KSS (MSX,
//!   Master System, Game Gear) and NSF and NSFE (NES/Famicom, with its
//!   expansion sound chips); a rip's tracks are started as orders
//!   ([`PROP_PLAY_START_ORDER_NUMBER`]), and its track information becomes
//!   metadata. This is the `gme` Cargo feature, on by default (as upstream
//!   builds it by default). **Game_Music_Emu is not zlib-licensed**:
//!   `sdl3-mixer-gme` is under `LGPL-2.1-or-later AND MIT`. With
//!   `default-features = false`, there is no game music decoder, as in an
//!   upstream build without libgme.
//!
//! As in the [`sdl3`] crate, the implementation is a line-by-line
//! translation and the API is designed for Rust: objects are owned handles
//! whose `Drop` destroys them (`MIX_Destroy*`), the `closeio` flags are gone
//! (a stream given to a track or decoder is owned by it from then on),
//! callbacks are closures, errors are [`Result`](sdl3::Result)s, and every
//! item names the C symbol it translates.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
// The translations keep upstream's code shape (its branches, constants,
// counters and arithmetic, including its quirks), which these lints would
// have rewritten.
#![allow(
    clippy::approx_constant,
    clippy::collapsible_match,
    clippy::eq_op,
    clippy::erasing_op,
    clippy::excessive_precision,
    clippy::explicit_counter_loop,
    clippy::identity_op,
    clippy::if_same_then_else,
    clippy::implicit_saturating_sub,
    clippy::manual_clamp,
    clippy::manual_is_multiple_of,
    clippy::manual_rotate,
    clippy::needless_option_as_deref,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::precedence,
    clippy::single_match
)]

mod decoder_aiff;
mod decoder_au;
mod decoder_drflac;
mod decoder_drmp3;
mod decoder_flac;
#[cfg(feature = "fluidsynth")]
mod decoder_fluidsynth;
#[cfg(feature = "gme")]
mod decoder_gme;
#[cfg(feature = "mpg123")]
mod decoder_mpg123;
mod decoder_opus;
mod decoder_raw;
mod decoder_sinewave;
mod decoder_stb_vorbis;
#[cfg(feature = "timidity")]
mod decoder_timidity;
mod decoder_voc;
mod decoder_vorbis;
mod decoder_wav;
mod decoder_wavpack;
mod decoder_xmp;
mod dr_flac;
mod dr_mp3;
mod flac;
mod internal;
mod metadata_tags;
mod mixer;
mod ogg;
mod opus;
mod opusfile;
mod spatialization;
mod stb_vorbis;
mod vorbis;
mod wavpack;
mod xmp;

pub use mixer::{
    audio_decoder, frames_to_ms, init, ms_to_frames, num_audio_decoders, quit, version, Audio,
    AudioDecoder, Group, GroupMixCallback, Mixer, MixerLock, PostMixCallback, Track,
    TrackMixCallback, TrackStoppedCallback,
};

/// The major version of SDL_mixer this crate translates.
/// Translation of `SDL_MIXER_MAJOR_VERSION`.
pub const MAJOR_VERSION: u16 = 3;
/// The minor version. Translation of `SDL_MIXER_MINOR_VERSION`.
pub const MINOR_VERSION: u16 = 3;
/// The micro (patch) version. Translation of `SDL_MIXER_MICRO_VERSION`.
pub const MICRO_VERSION: u16 = 0;

/// The SDL_mixer version this crate translates. Translation of
/// `SDL_MIXER_VERSION` (and, with [`Version::at_least`](sdl3::Version::at_least),
/// of `SDL_MIXER_VERSION_ATLEAST()`).
pub const VERSION: sdl3::Version = sdl3::Version::new(MAJOR_VERSION, MINOR_VERSION, MICRO_VERSION);

/// The upstream SDL_mixer revision this translation was made from.
pub const REVISION: &str = "SDL_mixer-3.3.0-df66ae893c91b0f6fa5d026a195bb278213d007d";

/// The SDL audio device the mixer plays on (a number; 0 for a mixer from
/// [`Mixer::new`]). Translation of `MIX_PROP_MIXER_DEVICE_NUMBER`.
pub const PROP_MIXER_DEVICE_NUMBER: &str = "SDL_mixer.mixer.device";

/// Translation of `MIX_PROP_AUDIO_LOAD_IOSTREAM_POINTER` (here the stream is
/// an argument of [`Audio::load_with_properties`]).
pub const PROP_AUDIO_LOAD_IOSTREAM_POINTER: &str = "SDL_mixer.audio.load.iostream";
/// Translation of `MIX_PROP_AUDIO_LOAD_CLOSEIO_BOOLEAN` (here the caller
/// owns the stream, so it is ignored).
pub const PROP_AUDIO_LOAD_CLOSEIO_BOOLEAN: &str = "SDL_mixer.audio.load.closeio";
/// Decode the whole file up front (a boolean, default `false`).
/// Translation of `MIX_PROP_AUDIO_LOAD_PREDECODE_BOOLEAN`.
pub const PROP_AUDIO_LOAD_PREDECODE_BOOLEAN: &str = "SDL_mixer.audio.load.predecode";
/// Translation of `MIX_PROP_AUDIO_LOAD_PREFERRED_MIXER_POINTER` (here the
/// mixer is an argument of [`Audio::load_with_properties`]).
pub const PROP_AUDIO_LOAD_PREFERRED_MIXER_POINTER: &str = "SDL_mixer.audio.load.preferred_mixer";
/// Don't look for ID3, APE, Lyrics3 and MusicMatch tags at the ends of the
/// file (a boolean, default `false`). Translation of
/// `MIX_PROP_AUDIO_LOAD_SKIP_METADATA_TAGS_BOOLEAN`.
pub const PROP_AUDIO_LOAD_SKIP_METADATA_TAGS_BOOLEAN: &str =
    "SDL_mixer.audio.load.skip_metadata_tags";
/// Ignore the loop points a file might define (a boolean, default
/// `false`). Translation of `MIX_PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN`.
pub const PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN: &str = "SDL_mixer.audio.load.ignore_loops";
/// The decoder to use (a string such as `"WAV"`; see [`audio_decoder`]),
/// and, after loading, the decoder used. Translation of
/// `MIX_PROP_AUDIO_DECODER_STRING`.
pub const PROP_AUDIO_DECODER_STRING: &str = "SDL_mixer.audio.decoder";

/// Translation of `MIX_PROP_METADATA_TITLE_STRING`.
pub const PROP_METADATA_TITLE_STRING: &str = "SDL_mixer.metadata.title";
/// Translation of `MIX_PROP_METADATA_ARTIST_STRING`.
pub const PROP_METADATA_ARTIST_STRING: &str = "SDL_mixer.metadata.artist";
/// Translation of `MIX_PROP_METADATA_ALBUM_STRING`.
pub const PROP_METADATA_ALBUM_STRING: &str = "SDL_mixer.metadata.album";
/// Translation of `MIX_PROP_METADATA_COPYRIGHT_STRING`.
pub const PROP_METADATA_COPYRIGHT_STRING: &str = "SDL_mixer.metadata.copyright";
/// Translation of `MIX_PROP_METADATA_TRACK_NUMBER`.
pub const PROP_METADATA_TRACK_NUMBER: &str = "SDL_mixer.metadata.track";
/// Translation of `MIX_PROP_METADATA_TOTAL_TRACKS_NUMBER`.
pub const PROP_METADATA_TOTAL_TRACKS_NUMBER: &str = "SDL_mixer.metadata.total_tracks";
/// Translation of `MIX_PROP_METADATA_YEAR_NUMBER`.
pub const PROP_METADATA_YEAR_NUMBER: &str = "SDL_mixer.metadata.year";
/// Translation of `MIX_PROP_METADATA_DURATION_FRAMES_NUMBER`.
pub const PROP_METADATA_DURATION_FRAMES_NUMBER: &str = "SDL_mixer.metadata.duration_frames";
/// Translation of `MIX_PROP_METADATA_DURATION_INFINITE_BOOLEAN`.
pub const PROP_METADATA_DURATION_INFINITE_BOOLEAN: &str = "SDL_mixer.metadata.duration_infinite";

/// The duration of the audio isn't known. Translation of `MIX_DURATION_UNKNOWN`.
pub const DURATION_UNKNOWN: i64 = -1;
/// The audio plays forever (it loops infinitely, or is generated).
/// Translation of `MIX_DURATION_INFINITE`.
pub const DURATION_INFINITE: i64 = -2;

/// Loop this many times after the first play; -1 loops forever (a number,
/// default 0). Translation of `MIX_PROP_PLAY_LOOPS_NUMBER`.
pub const PROP_PLAY_LOOPS_NUMBER: &str = "SDL_mixer.play.loops";
/// Stop at this sample frame (a number). Translation of `MIX_PROP_PLAY_MAX_FRAME_NUMBER`.
pub const PROP_PLAY_MAX_FRAME_NUMBER: &str = "SDL_mixer.play.max_frame";
/// Stop after this many milliseconds (a number). Translation of
/// `MIX_PROP_PLAY_MAX_MILLISECONDS_NUMBER`.
pub const PROP_PLAY_MAX_MILLISECONDS_NUMBER: &str = "SDL_mixer.play.max_milliseconds";
/// Start at this sample frame (a number). Translation of `MIX_PROP_PLAY_START_FRAME_NUMBER`.
pub const PROP_PLAY_START_FRAME_NUMBER: &str = "SDL_mixer.play.start_frame";
/// Start at this millisecond (a number). Translation of
/// `MIX_PROP_PLAY_START_MILLISECOND_NUMBER`.
pub const PROP_PLAY_START_MILLISECOND_NUMBER: &str = "SDL_mixer.play.start_millisecond";
/// Start at this order of a module (a number; for the `XMP` decoder, ignored
/// by the others). Translation of `MIX_PROP_PLAY_START_ORDER_NUMBER`.
pub const PROP_PLAY_START_ORDER_NUMBER: &str = "SDL_mixer.play.start_order";
/// Loops start at this sample frame (a number). Translation of
/// `MIX_PROP_PLAY_LOOP_START_FRAME_NUMBER`.
pub const PROP_PLAY_LOOP_START_FRAME_NUMBER: &str = "SDL_mixer.play.loop_start_frame";
/// Loops start at this millisecond (a number). Translation of
/// `MIX_PROP_PLAY_LOOP_START_MILLISECOND_NUMBER`.
pub const PROP_PLAY_LOOP_START_MILLISECOND_NUMBER: &str = "SDL_mixer.play.loop_start_millisecond";
/// Fade in over this many sample frames (a number). Translation of
/// `MIX_PROP_PLAY_FADE_IN_FRAMES_NUMBER`.
pub const PROP_PLAY_FADE_IN_FRAMES_NUMBER: &str = "SDL_mixer.play.fade_in_frames";
/// Fade in over this many milliseconds (a number). Translation of
/// `MIX_PROP_PLAY_FADE_IN_MILLISECONDS_NUMBER`.
pub const PROP_PLAY_FADE_IN_MILLISECONDS_NUMBER: &str = "SDL_mixer.play.fade_in_milliseconds";
/// Start the fade in from this gain, 0.0 to 1.0 (a float). Translation of
/// `MIX_PROP_PLAY_FADE_IN_START_GAIN_FLOAT`.
pub const PROP_PLAY_FADE_IN_START_GAIN_FLOAT: &str = "SDL_mixer.play.fade_in_start_gain";
/// Append this many sample frames of silence (a number). Translation of
/// `MIX_PROP_PLAY_APPEND_SILENCE_FRAMES_NUMBER`.
pub const PROP_PLAY_APPEND_SILENCE_FRAMES_NUMBER: &str = "SDL_mixer.play.append_silence_frames";
/// Append this many milliseconds of silence (a number). Translation of
/// `MIX_PROP_PLAY_APPEND_SILENCE_MILLISECONDS_NUMBER`.
pub const PROP_PLAY_APPEND_SILENCE_MILLISECONDS_NUMBER: &str =
    "SDL_mixer.play.append_silence_milliseconds";
/// Stop the track when its input runs out (a boolean, default `true`).
/// Translation of `MIX_PROP_PLAY_HALT_WHEN_EXHAUSTED_BOOLEAN`.
pub const PROP_PLAY_HALT_WHEN_EXHAUSTED_BOOLEAN: &str = "SDL_mixer.play.halt_when_exhausted";

/// The gains of a track forced to stereo. Translation of `MIX_StereoGains`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct StereoGains {
    /// left channel gain
    pub left: f32,
    /// right channel gain
    pub right: f32,
}

/// A position in 3D space, for [`Track::set_3d_position`]. Translation of
/// `MIX_Point3D`.
///
/// The listener is at the origin, looking down the negative Z axis, with
/// positive Y up (OpenAL's default orientation).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Point3D {
    /// X coordinate (negative left, positive right).
    pub x: f32,
    /// Y coordinate (negative down, positive up).
    pub y: f32,
    /// Z coordinate (negative forward, positive back).
    pub z: f32,
}

#[cfg(test)]
mod tests;
