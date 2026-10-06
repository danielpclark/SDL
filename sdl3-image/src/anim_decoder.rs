// Rust translation of src/IMG_anim_decoder.c and src/IMG_anim_decoder.h
// from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Animation decoders: frames one at a time from a file or a stream, by
//! format (GIF, ANI and WebP here; APNG and AVIF need their libraries,
//! which this crate doesn't have, as an upstream build without them), with
//! a single-frame decoder for every other format; and whole animations
//! decoded into an [`Animation`](crate::Animation).

use std::path::Path;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::stdlib::string::strcasecmp;
use sdl3::video::Surface;

use crate::ani::{create_ani_animation_decoder, AniDecoderContext};
use crate::gif::{create_gif_animation_decoder, GifContext};
use crate::img::{timebase_duration, Animation};
use crate::webp::{create_webp_animation_decoder, WebpDecoderContext};

/// The file to decode (a string). Translation of
/// `IMG_PROP_ANIMATION_DECODER_CREATE_FILENAME_STRING`.
pub const PROP_ANIMATION_DECODER_CREATE_FILENAME_STRING: &str =
    "SDL_image.animation_decoder.create.filename";
/// Translation of `IMG_PROP_ANIMATION_DECODER_CREATE_IOSTREAM_POINTER`
/// (here the stream is an argument of [`AnimationDecoder::with_properties`]).
pub const PROP_ANIMATION_DECODER_CREATE_IOSTREAM_POINTER: &str =
    "SDL_image.animation_decoder.create.iostream";
/// Translation of
/// `IMG_PROP_ANIMATION_DECODER_CREATE_IOSTREAM_AUTOCLOSE_BOOLEAN` (here the
/// caller owns the stream).
pub const PROP_ANIMATION_DECODER_CREATE_IOSTREAM_AUTOCLOSE_BOOLEAN: &str =
    "SDL_image.animation_decoder.create.iostream.autoclose";
/// The format, a file extension such as `"gif"` (a string). Translation of
/// `IMG_PROP_ANIMATION_DECODER_CREATE_TYPE_STRING`.
pub const PROP_ANIMATION_DECODER_CREATE_TYPE_STRING: &str =
    "SDL_image.animation_decoder.create.type";
/// The numerator of the time base of the durations (a number, default 1).
/// Translation of `IMG_PROP_ANIMATION_DECODER_CREATE_TIMEBASE_NUMERATOR_NUMBER`.
pub const PROP_ANIMATION_DECODER_CREATE_TIMEBASE_NUMERATOR_NUMBER: &str =
    "SDL_image.animation_decoder.create.timebase.numerator";
/// The denominator of the time base (a number, default 1000).
/// Translation of `IMG_PROP_ANIMATION_DECODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER`.
pub const PROP_ANIMATION_DECODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER: &str =
    "SDL_image.animation_decoder.create.timebase.denominator";
/// Translation of `IMG_PROP_ANIMATION_DECODER_CREATE_AVIF_MAX_THREADS_NUMBER`
/// (for the AVIF decoder, not in this crate).
pub const PROP_ANIMATION_DECODER_CREATE_AVIF_MAX_THREADS_NUMBER: &str =
    "SDL_image.animation_decoder.create.avif.max_threads";
/// Translation of `IMG_PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_INCREMENTAL_BOOLEAN`
/// (for the AVIF decoder, not in this crate).
pub const PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_INCREMENTAL_BOOLEAN: &str =
    "SDL_image.animation_decoder.create.avif.allow_incremental";
/// Translation of `IMG_PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_PROGRESSIVE_BOOLEAN`
/// (for the AVIF decoder, not in this crate).
pub const PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_PROGRESSIVE_BOOLEAN: &str =
    "SDL_image.animation_decoder.create.avif.allow_progressive";
/// The GIF encoder's transparent color index (a number; despite its name,
/// an encoder property). Translation of
/// `IMG_PROP_ANIMATION_DECODER_CREATE_GIF_TRANSPARENT_COLOR_INDEX_NUMBER`.
pub const PROP_ANIMATION_DECODER_CREATE_GIF_TRANSPARENT_COLOR_INDEX_NUMBER: &str =
    "SDL_image.animation_encoder.create.gif.transparent_color_index";
/// The GIF encoder's number of colors, a power of 2 from 2 to 256 (a
/// number; despite its name, an encoder property). Translation of
/// `IMG_PROP_ANIMATION_DECODER_CREATE_GIF_NUM_COLORS_NUMBER`.
pub const PROP_ANIMATION_DECODER_CREATE_GIF_NUM_COLORS_NUMBER: &str =
    "SDL_image.animation_encoder.create.gif.num_colors";

/// Don't read or write metadata (a boolean). Translation of
/// `IMG_PROP_METADATA_IGNORE_PROPS_BOOLEAN`.
pub const PROP_METADATA_IGNORE_PROPS_BOOLEAN: &str = "SDL_image.metadata.ignore_props";
/// Translation of `IMG_PROP_METADATA_DESCRIPTION_STRING`.
pub const PROP_METADATA_DESCRIPTION_STRING: &str = "SDL_image.metadata.description";
/// Translation of `IMG_PROP_METADATA_COPYRIGHT_STRING`.
pub const PROP_METADATA_COPYRIGHT_STRING: &str = "SDL_image.metadata.copyright";
/// Translation of `IMG_PROP_METADATA_TITLE_STRING`.
pub const PROP_METADATA_TITLE_STRING: &str = "SDL_image.metadata.title";
/// Translation of `IMG_PROP_METADATA_AUTHOR_STRING`.
pub const PROP_METADATA_AUTHOR_STRING: &str = "SDL_image.metadata.author";
/// Translation of `IMG_PROP_METADATA_CREATION_TIME_STRING`.
pub const PROP_METADATA_CREATION_TIME_STRING: &str = "SDL_image.metadata.creation_time";
/// Translation of `IMG_PROP_METADATA_FRAME_COUNT_NUMBER`.
pub const PROP_METADATA_FRAME_COUNT_NUMBER: &str = "SDL_image.metadata.frame_count";
/// Translation of `IMG_PROP_METADATA_LOOP_COUNT_NUMBER`.
pub const PROP_METADATA_LOOP_COUNT_NUMBER: &str = "SDL_image.metadata.loop_count";

/// Whether the GIF decoder returns its frames as decoded, without a canvas
/// (for [`load_gif_io`](crate::load_gif_io)). Translation of IMG_gif.c's
/// `IMG_PROP_ANIMATION_DECODER_CREATE_GIF_SINGLE_IMAGE_BOOLEAN`.
pub(crate) const PROP_ANIMATION_DECODER_CREATE_GIF_SINGLE_IMAGE_BOOLEAN: &str =
    "SDL_image.animation_decoder.create.gif.single_image";

/// The status of an animation decoder. Translation of
/// `IMG_AnimationDecoderStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationDecoderStatus {
    /// The decoder is invalid. Translation of `IMG_DECODER_STATUS_INVALID`
    /// (which a Rust decoder never is).
    Invalid,
    /// The decoder is ready to decode the next frame. Translation of
    /// `IMG_DECODER_STATUS_OK`.
    Ok,
    /// The decoder failed to decode a frame. Translation of
    /// `IMG_DECODER_STATUS_FAILED`.
    Failed,
    /// No more frames available. Translation of
    /// `IMG_DECODER_STATUS_COMPLETE`.
    Complete,
}

/// A stream a decoder or encoder reads or writes: the caller's, or one it
/// opened from a file (which it closes).
pub(crate) enum Stream<'s, 'a> {
    Borrowed(&'s mut IoStream<'a>),
    Owned(IoStream<'a>),
}

impl<'a> Stream<'_, 'a> {
    pub(crate) fn io(&mut self) -> &mut IoStream<'a> {
        match self {
            Stream::Borrowed(io) => io,
            Stream::Owned(io) => io,
        }
    }
}

/// The part of `IMG_AnimationDecoder` the format decoders share: the
/// stream, its start, the status, time base, accumulated presentation time
/// and metadata properties.
pub(crate) struct DecoderCore<'s, 'a> {
    pub(crate) status: AnimationDecoderStatus,
    pub(crate) props: Properties,
    pub(crate) src: Stream<'s, 'a>,
    pub(crate) start: i64,
    pub(crate) timebase_numerator: i32,
    pub(crate) timebase_denominator: i32,
    pub(crate) accumulated_pts: u64,
}

impl<'a> DecoderCore<'_, 'a> {
    /// The stream.
    pub(crate) fn src(&mut self) -> &mut IoStream<'a> {
        self.src.io()
    }

    /// A duration in `1 / timebase_denominator` seconds in the decoder's
    /// time base, advancing its presentation time. Translation of
    /// `IMG_GetDecoderDuration()`.
    pub(crate) fn decoder_duration(&mut self, duration: u64, timebase_denominator: u64) -> u64 {
        let value = timebase_duration(
            self.accumulated_pts,
            duration,
            1,
            timebase_denominator,
            self.timebase_numerator as u64,
            self.timebase_denominator as u64,
        );
        self.accumulated_pts = self.accumulated_pts.wrapping_add(duration);
        value
    }
}

/// Translation of `struct IMG_AnimationDecoderContext`, one per decoder:
/// the single-frame one's (`type`, `frame_read`) or a format's.
pub(crate) enum DecoderContext {
    None,
    SingleFrame { type_: String, frame_read: bool },
    Gif(Box<GifContext>),
    Ani(Box<AniDecoderContext>),
    Webp(Box<WebpDecoderContext>),
}

/// A decoder of the frames of an animation, one at a time. Translation of
/// `IMG_AnimationDecoder`. Dropping it closes it (see [`close`](Self::close)).
pub struct AnimationDecoder<'s, 'a> {
    core: DecoderCore<'s, 'a>,
    ctx: DecoderContext,
}

impl std::fmt::Debug for AnimationDecoder<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format = match &self.ctx {
            DecoderContext::None => "none",
            DecoderContext::SingleFrame { .. } => "single frame",
            DecoderContext::Gif(_) => "gif",
            DecoderContext::Ani(_) => "ani",
            DecoderContext::Webp(_) => "webp",
        };
        f.debug_struct("AnimationDecoder")
            .field("status", &self.core.status)
            .field("format", &format)
            .finish_non_exhaustive()
    }
}

/// Translation of `IMG_SingleFrameDecoderReset()`.
fn single_frame_decoder_reset(d: &mut DecoderCore<'_, '_>, frame_read: &mut bool) -> Result<()> {
    let start = d.start;
    if d.src().seek(start, IoWhence::Set).ok() != Some(start) {
        return Err(Error::new("Failed to seek in the animation stream"));
    }

    *frame_read = false;
    Ok(())
}

/// Translation of `IMG_SingleFrameDecoderGetNextFrame()`.
fn single_frame_decoder_get_next_frame(
    d: &mut DecoderCore<'_, '_>,
    type_: &str,
    frame_read: &mut bool,
) -> Result<Option<(Surface<'static>, u64)>> {
    if *frame_read {
        d.status = AnimationDecoderStatus::Complete;
        return Ok(None);
    }

    let frame = crate::load_typed_io(d.src(), Some(type_))?;
    *frame_read = true;
    Ok(Some((frame, 0)))
}

/// Translation of `IMG_CreateSingleFrameAnimationDecoder()`.
fn create_single_frame_animation_decoder(type_: &str) -> DecoderContext {
    DecoderContext::SingleFrame {
        type_: type_.to_owned(),
        frame_read: false,
    }
}

/// The type of a file: what follows the last `.` of its name.
fn type_of_file(file: &str) -> Option<&str> {
    // Skip the '.' in the file extension
    file.rfind('.').map(|i| &file[i + 1..])
}

impl AnimationDecoder<'static, 'static> {
    /// Create a decoder for an animation file, of the format its extension
    /// names. Translation of `IMG_CreateAnimationDecoder()`.
    pub fn new(file: impl AsRef<Path>) -> Result<AnimationDecoder<'static, 'static>> {
        let file = file.as_ref();
        if file.as_os_str().is_empty() {
            return Err(Error::invalid_param("file"));
        }

        let props = Properties::new();
        props.set(
            PROP_ANIMATION_DECODER_CREATE_FILENAME_STRING,
            file.to_string_lossy().into_owned(),
        )?;
        AnimationDecoder::with_properties(None, &props)
    }
}

impl<'s, 'a> AnimationDecoder<'s, 'a> {
    /// Create a decoder for an animation in `src` (from its position), of
    /// the format `type_` names (a file extension such as `"gif"`; formats
    /// without an animation decoder get a single-frame one). Translation of
    /// `IMG_CreateAnimationDecoder_IO()`.
    pub fn from_io(src: &'s mut IoStream<'a>, type_: &str) -> Result<AnimationDecoder<'s, 'a>> {
        if type_.is_empty() {
            return Err(Error::invalid_param("type"));
        }

        let props = Properties::new();
        props.set(PROP_ANIMATION_DECODER_CREATE_TYPE_STRING, type_)?;
        AnimationDecoder::with_properties(Some(src), &props)
    }

    /// Create a decoder with properties: `src`, or the file named by
    /// [`PROP_ANIMATION_DECODER_CREATE_FILENAME_STRING`]; the type (else the
    /// file's extension); the time base; and the format decoders' own
    /// properties. Translation of `IMG_CreateAnimationDecoderWithProperties()`
    /// (the stream, a pointer property there, is an argument here).
    pub fn with_properties(
        src: Option<&'s mut IoStream<'a>>,
        props: &Properties,
    ) -> Result<AnimationDecoder<'s, 'a>> {
        AnimationDecoder::with_properties_and_fallback(src, props, true)
    }

    /// [`with_properties`](Self::with_properties), with or without the
    /// fallback to the single-frame decoder when the format's decoder
    /// can't be created (without it, that is an error).
    pub(crate) fn with_properties_and_fallback(
        src: Option<&'s mut IoStream<'a>>,
        props: &Properties,
        fallback: bool,
    ) -> Result<AnimationDecoder<'s, 'a>> {
        let file = props.get_string(PROP_ANIMATION_DECODER_CREATE_FILENAME_STRING);
        let mut type_ = props.get_string(PROP_ANIMATION_DECODER_CREATE_TYPE_STRING);
        // FIXME (upstream): the time base is read from the encoder's
        // properties, not the decoder's.
        let timebase_numerator = props
            .get_number(
                crate::anim_encoder::PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_NUMERATOR_NUMBER,
            )
            .unwrap_or(1) as i32;
        let timebase_denominator = props
            .get_number(
                crate::anim_encoder::PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER,
            )
            .unwrap_or(1000) as i32;

        if type_.as_deref().is_none_or(str::is_empty) {
            type_ = file.as_deref().and_then(type_of_file).map(str::to_owned);
            if type_.as_deref().is_none_or(str::is_empty) {
                return Err(Error::new("Couldn't determine file type"));
            }
        }
        let type_ = type_.unwrap_or_default();

        if timebase_numerator <= 0 {
            return Err(Error::new("Time base numerator must be > 0"));
        }

        if timebase_denominator <= 0 {
            return Err(Error::new("Time base denominator must be > 0"));
        }

        let src = match src {
            Some(src) => Stream::Borrowed(src),
            None => {
                let Some(file) = &file else {
                    return Err(Error::new("No input properties set"));
                };
                Stream::Owned(IoStream::from_file(file, "rb")?)
            }
        };

        let mut decoder = AnimationDecoder {
            core: DecoderCore {
                status: AnimationDecoderStatus::Ok,
                props: Properties::new(),
                src,
                start: 0,
                timebase_numerator,
                timebase_denominator,
                accumulated_pts: 0,
            },
            ctx: DecoderContext::None,
        };
        decoder.core.start = decoder.core.src().tell().unwrap_or(-1);

        let is = |name: &str| strcasecmp(&type_, name).is_eq();
        let d = &mut decoder.core;
        let result = if is("ani") {
            create_ani_animation_decoder(d, props).map(DecoderContext::Ani)
        } else if is("apng") || is("png") {
            Err(Error::new("SDL_image not built against libpng."))
        } else if is("avifs") || is("avif") {
            Err(Error::new("SDL_image built without AVIF animation support"))
        } else if is("gif") {
            create_gif_animation_decoder(d, props).map(DecoderContext::Gif)
        } else if is("webp") {
            create_webp_animation_decoder(d, props).map(DecoderContext::Webp)
        } else {
            // (no decoder for the type: the single-frame one below)
            Err(Error::unsupported())
        };

        decoder.ctx = match result {
            Ok(ctx) => ctx,
            Err(e) if !fallback => return Err(e),
            Err(_) => {
                let start = decoder.core.start;
                if decoder.core.src().seek(start, IoWhence::Set).ok() != Some(start) {
                    return Err(Error::new("Couldn't seek in the animation stream"));
                }

                create_single_frame_animation_decoder(&type_)
            }
        };

        Ok(decoder)
    }

    /// The decoder's metadata (a format's loop count, comment, title,
    /// author, ...). Translation of `IMG_GetAnimationDecoderProperties()`.
    pub fn properties(&self) -> &Properties {
        &self.core.props
    }

    /// Decode the next frame and its duration (in the decoder's time base):
    /// `Ok(None)` when there are no more frames (the status is then
    /// [`Complete`](AnimationDecoderStatus::Complete)); on an error the
    /// status is [`Failed`](AnimationDecoderStatus::Failed). Translation of
    /// `IMG_GetAnimationDecoderFrame()`.
    pub fn get_frame(&mut self) -> Result<Option<(Surface<'static>, u64)>> {
        // Reset the status before trying to get the next frame
        self.core.status = AnimationDecoderStatus::Ok;

        let d = &mut self.core;
        let result = match &mut self.ctx {
            DecoderContext::None => Err(Error::invalid_param("decoder")),
            DecoderContext::SingleFrame { type_, frame_read } => {
                single_frame_decoder_get_next_frame(d, type_, frame_read)
            }
            DecoderContext::Gif(ctx) => ctx.get_next_frame(d),
            DecoderContext::Ani(ctx) => ctx.get_next_frame(d),
            DecoderContext::Webp(ctx) => ctx.get_next_frame(d),
        };

        // (the formats return Ok(None) with the COMPLETE status, where
        // upstream's return false with it)
        if result.is_err() {
            self.core.status = AnimationDecoderStatus::Failed;
        }
        result
    }

    /// The decoder's status. Translation of
    /// `IMG_GetAnimationDecoderStatus()`.
    pub fn status(&self) -> AnimationDecoderStatus {
        self.core.status
    }

    /// Go back to the first frame. Translation of
    /// `IMG_ResetAnimationDecoder()`.
    pub fn reset(&mut self) -> Result<()> {
        let d = &mut self.core;
        match &mut self.ctx {
            DecoderContext::None => Err(Error::invalid_param("decoder")),
            DecoderContext::SingleFrame { frame_read, .. } => {
                single_frame_decoder_reset(d, frame_read)
            }
            DecoderContext::Gif(ctx) => ctx.reset(d),
            DecoderContext::Ani(ctx) => {
                ctx.reset();
                Ok(())
            }
            DecoderContext::Webp(ctx) => {
                ctx.reset();
                Ok(())
            }
        }
    }

    /// Close the decoder, and the stream it opened, if any. Translation of
    /// `IMG_CloseAnimationDecoder()`.
    pub fn close(mut self) -> Result<()> {
        self.close_internal()
    }

    fn close_internal(&mut self) -> Result<()> {
        self.ctx = DecoderContext::None;
        let src = std::mem::replace(
            &mut self.core.src,
            Stream::Owned(IoStream::from_const_mem(&[])),
        );
        match src {
            Stream::Owned(io) => io.close(),
            Stream::Borrowed(_) => Ok(()),
        }
    }
}

impl Drop for AnimationDecoder<'_, '_> {
    fn drop(&mut self) {
        let _ = self.close_internal();
    }
}

/// Decode an animation in `src` of the format `format` names, at most
/// `max_frames` frames (0 for all); the decoder's metadata goes to the
/// first frame's properties. Translation of `IMG_DecodeAsAnimation()`.
pub(crate) fn decode_as_animation(
    src: &mut IoStream<'_>,
    format: &str,
    max_frames: i32,
) -> Result<Animation> {
    decode_as_animation_and_fallback(src, format, max_frames, true)
}

/// [`decode_as_animation`] without the single-frame decoder's fallback
/// (for a loader that the fallback would call again).
pub(crate) fn decode_as_animation_without_fallback(
    src: &mut IoStream<'_>,
    format: &str,
    max_frames: i32,
) -> Result<Animation> {
    decode_as_animation_and_fallback(src, format, max_frames, false)
}

/// The body of `IMG_DecodeAsAnimation()`.
fn decode_as_animation_and_fallback(
    src: &mut IoStream<'_>,
    format: &str,
    max_frames: i32,
    fallback: bool,
) -> Result<Animation> {
    // (IMG_CreateAnimationDecoder_IO())
    if format.is_empty() {
        return Err(Error::invalid_param("type"));
    }
    let props = Properties::new();
    props.set(PROP_ANIMATION_DECODER_CREATE_TYPE_STRING, format)?;
    let mut decoder = AnimationDecoder::with_properties_and_fallback(Some(src), &props, fallback)?;

    // We do not rely on the metadata for the count of available frames because some
    // formats like GIF only supports continuous decoding and doesn't have any data that
    // states the total available frames inside the binary data.
    //
    // For this reason, we will decode frames until we reach the end of the stream or
    // we reach the maximum number of frames specified by the caller.
    let mut frames: Vec<Surface<'static>> = Vec::new();
    let mut delays: Vec<u64> = Vec::new();

    loop {
        if max_frames > 0 && frames.len() >= max_frames as usize {
            break;
        }

        match decoder.get_frame()? {
            Some((next_frame, duration)) => {
                frames.push(next_frame);
                delays.push(duration);
            }
            // Decoding complete
            None => break,
        }
    }

    // Copy animation metadata to the first surface
    if let Some(first) = frames.first_mut() {
        let src_props = decoder.properties().clone();
        let _ = first.properties().copy_from(&src_props);
    }

    let _ = decoder.close();

    if frames.is_empty() {
        // FIXME (upstream): "Animation didn't contain any frames" is set, but
        // the error path closes the already-closed (NULL) decoder, which
        // replaces it with an invalid-parameter error.
        return Err(Error::invalid_param("decoder"));
    }

    Ok(Animation {
        w: frames[0].width(),
        h: frames[0].height(),
        delays: delays.iter().map(|&d| d as i32).collect(),
        frames,
    })
}

/// Load a Windows animated cursor (`.ani`) as an animation.
/// Translation of `IMG_LoadANIAnimation_IO()`.
pub fn load_ani_animation_io(src: &mut IoStream<'_>) -> Result<Animation> {
    decode_as_animation(src, "ani", 0)
}

/// Load an animated PNG as an animation: without libpng, as upstream built
/// without it, the PNG's image as a single frame. Translation of
/// `IMG_LoadAPNGAnimation_IO()`.
pub fn load_apng_animation_io(src: &mut IoStream<'_>) -> Result<Animation> {
    decode_as_animation(src, "png", 0)
}

/// Load an AVIF image sequence as an animation: AVIF isn't decoded here
/// (upstream needs libavif), so this fails as an upstream build without
/// it does. Translation of `IMG_LoadAVIFAnimation_IO()`.
pub fn load_avif_animation_io(src: &mut IoStream<'_>) -> Result<Animation> {
    decode_as_animation(src, "avifs", 0)
}

/// Load a GIF animation: every frame composited on the canvas, with its
/// delay. Translation of `IMG_LoadGIFAnimation_IO()`.
pub fn load_gif_animation_io(src: &mut IoStream<'_>) -> Result<Animation> {
    decode_as_animation(src, "gif", 0)
}

/// Load a WebP animation: every frame composited on the canvas, with its
/// duration. Translation of `IMG_LoadWEBPAnimation_IO()`.
pub fn load_webp_animation_io(src: &mut IoStream<'_>) -> Result<Animation> {
    decode_as_animation(src, "webp", 0)
}
